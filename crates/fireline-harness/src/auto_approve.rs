use crate::approval::approval_resolution_envelope_with_trace;
use crate::durable_subscriber::{
    ActiveSubscriber, CompletionKey, DurableSubscriber, HandlerOutcome, StreamEnvelope,
    TraceContext,
};
use async_trait::async_trait;
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset, Producer};
use fireline_acp_ids::{RequestId, SessionId};
use sacp::{ConnectTo, Proxy};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct AutoApproveConfig {
    pub resolved_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AutoApproveSubscriber {
    resolved_by: String,
}

impl AutoApproveSubscriber {
    #[must_use]
    pub fn new(config: AutoApproveConfig) -> Self {
        Self {
            resolved_by: config
                .resolved_by
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "auto_approve".to_string()),
        }
    }
}

#[derive(Clone)]
pub struct AutoApproveSubscriberComponent {
    subscriber: AutoApproveSubscriber,
    state_stream_url: String,
    state_producer: Producer,
}

impl AutoApproveSubscriberComponent {
    #[must_use]
    pub fn new(
        config: AutoApproveConfig,
        state_stream_url: impl Into<String>,
        state_producer: Producer,
    ) -> Self {
        Self {
            subscriber: AutoApproveSubscriber::new(config),
            state_stream_url: state_stream_url.into(),
            state_producer,
        }
    }

    async fn run(self) -> anyhow::Result<()> {
        let client = DurableStreamsClient::new();
        let stream = client.stream(&self.state_stream_url);
        let mut reader = stream
            .read()
            .offset(Offset::Beginning)
            .live(LiveMode::Sse)
            .build()?;
        let mut log: Vec<StreamEnvelope> = Vec::new();

        while let Some(chunk) = reader.next_chunk().await? {
            if chunk.data.is_empty() {
                continue;
            }

            let events: Vec<Value> = serde_json::from_slice(&chunk.data)?;
            let mut chunk_log = Vec::new();
            for event in events {
                match StreamEnvelope::from_json(event) {
                    Ok(envelope) => chunk_log.push(envelope),
                    Err(error) => {
                        tracing::warn!(%error, "auto_approve skipped malformed stream envelope");
                    }
                }
            }
            log.extend(chunk_log.iter().cloned());

            for envelope in chunk_log {
                let Some(event) = self.subscriber.matches(&envelope) else {
                    continue;
                };
                if self.subscriber.is_completed(&event, &log) {
                    continue;
                }

                match self.subscriber.handle(event).await {
                    HandlerOutcome::Completed(completion) => {
                        let completion_envelope = self.emit_completion(&completion).await?;
                        log.push(completion_envelope);
                    }
                    HandlerOutcome::RetryTransient(error) => {
                        tracing::warn!(%error, "auto_approve transient failure");
                    }
                    HandlerOutcome::Failed(error) => {
                        tracing::warn!(%error, "auto_approve terminal failure");
                    }
                }
            }
        }

        Ok(())
    }

    async fn emit_completion(
        &self,
        completion: &ApprovalResolvedCompletion,
    ) -> anyhow::Result<StreamEnvelope> {
        let envelope = approval_resolution_envelope_with_trace(
            completion.session_id.clone(),
            completion.request_id.clone(),
            completion.allow,
            completion.resolved_by.clone(),
            completion.meta.clone(),
        )?;
        self.state_producer.append_json(&envelope);
        self.state_producer.flush().await?;
        Ok(envelope)
    }
}

impl ConnectTo<sacp::Conductor> for AutoApproveSubscriberComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let runner = self.clone();
        tokio::spawn(async move {
            if let Err(error) = runner.run().await {
                tracing::warn!(%error, "auto_approve subscriber stopped");
            }
        });

        sacp::Proxy
            .builder()
            .name("fireline-auto-approve")
            .connect_to(client)
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestEvent {
    kind: String,
    session_id: SessionId,
    request_id: RequestId,
    #[serde(default)]
    reason: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    meta: Option<TraceContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalResolvedCompletion {
    session_id: SessionId,
    request_id: RequestId,
    allow: bool,
    resolved_by: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    meta: Option<TraceContext>,
}

impl DurableSubscriber for AutoApproveSubscriber {
    type Event = PermissionRequestEvent;
    type Completion = ApprovalResolvedCompletion;

    fn name(&self) -> &str {
        "auto_approve"
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        let event: PermissionRequestEvent = envelope.value_as()?;
        (event.kind == "permission_request").then_some(event)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        CompletionKey::prompt(event.session_id.clone(), event.request_id.clone())
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log.iter().any(|envelope| {
            let Some(value) = envelope.value.as_ref() else {
                return false;
            };
            if value.get("kind").and_then(Value::as_str) != Some("approval_resolved") {
                return false;
            }
            let Some(session_id) = value
                .get("sessionId")
                .and_then(|value| value.as_str().map(|text| SessionId::from(text.to_string())))
            else {
                return false;
            };
            let Some(request_id) = value
                .get("requestId")
                .and_then(|value| serde_json::from_value::<RequestId>(value.clone()).ok())
            else {
                return false;
            };
            session_id == event.session_id && request_id == event.request_id
        })
    }
}

#[async_trait]
impl ActiveSubscriber for AutoApproveSubscriber {
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion> {
        HandlerOutcome::Completed(ApprovalResolvedCompletion {
            session_id: event.session_id,
            request_id: event.request_id,
            allow: true,
            resolved_by: self.resolved_by.clone(),
            meta: event.meta,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval_resolution_envelope;
    use crate::durable_subscriber::{DurableSubscriberDriver, SubscriberMode, SubscriberRegistration};

    #[test]
    fn auto_approve_uses_same_completion_spine_as_manual_resolution() {
        let subscriber = AutoApproveSubscriber::new(AutoApproveConfig::default());
        let permission_request = StreamEnvelope::from_json(serde_json::json!({
            "type": "permission",
            "key": "session-1:request-1",
            "headers": { "operation": "insert" },
            "value": {
                "kind": "permission_request",
                "sessionId": "session-1",
                "requestId": "request-1",
                "reason": "test policy"
            }
        }))
        .expect("decode permission_request envelope");

        let event = subscriber
            .matches(&permission_request)
            .expect("permission_request should match auto-approve");
        let manual_resolution = crate::approval::approval_resolution_envelope(
            SessionId::from("session-1"),
            RequestId::from("request-1".to_string()),
            true,
            "manual-approver".to_string(),
        )
        .expect("build manual completion envelope");

        assert_eq!(
            subscriber.completion_key(&event),
            manual_resolution
                .completion_key()
                .expect("completion key from manual resolution"),
            "DSV-01 CompletionKeyUnique: auto and manual approval paths must key completion on the same canonical (session_id, request_id) tuple",
        );
        assert!(
            subscriber.is_completed(&event, &[manual_resolution]),
            "DSV-02 ReplayIdempotent: an existing manual approval_resolved envelope must suppress auto-approve for the same completion key during replay",
        );
    }

    #[test]
    fn auto_approve_replay_skips_duplicate_completion_when_manual_resolution_exists() {
        let subscriber = AutoApproveSubscriber::new(AutoApproveConfig::default());
        let mut driver = DurableSubscriberDriver::new();
        driver.register_active(subscriber.clone());
        assert_eq!(
            driver.registrations(),
            vec![SubscriberRegistration {
                name: "auto_approve".to_string(),
                mode: SubscriberMode::Active,
            }],
            "DSV-02 ReplayIdempotent: a fresh driver must register the same auto_approve profile before replay suppression is evaluated",
        );

        let permission_request = StreamEnvelope::from_json(serde_json::json!({
            "type": "permission",
            "key": "session-1:request-1",
            "headers": { "operation": "insert" },
            "value": {
                "kind": "permission_request",
                "sessionId": "session-1",
                "requestId": "request-1",
                "reason": "test policy"
            }
        }))
        .expect("decode permission_request envelope");
        let manual_resolution = approval_resolution_envelope(
            SessionId::from("session-1"),
            RequestId::from("request-1".to_string()),
            true,
            "manual-approver".to_string(),
        )
        .expect("build manual completion envelope");

        let event = subscriber
            .matches(&permission_request)
            .expect("DSV-02 ReplayIdempotent: permission_request should still match on replay");
        let replay_log = vec![manual_resolution];

        assert!(
            subscriber.is_completed(&event, &replay_log),
            "DSV-02 ReplayIdempotent: replay with a preexisting approval_resolved completion must mark the auto_approve event complete",
        );
        let should_emit = !subscriber.is_completed(&event, &replay_log);
        assert!(
            !should_emit,
            "DSV-02 ReplayIdempotent: replay should skip a second auto_approve emission rather than minting a duplicate approval_resolved envelope",
        );
    }
}
