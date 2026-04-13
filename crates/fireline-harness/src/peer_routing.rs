use std::sync::Arc;

use async_trait::async_trait;
use fireline_acp_ids::{RequestId, SessionId, ToolCallId};
use sacp::schema::{SessionUpdate, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    ActiveSubscriber, CompletionKey, DurableSubscriber, HandlerOutcome, RetryPolicy,
    StreamEnvelope, TraceContext,
};

pub const PEER_DELIVERY_ACK_ENTITY_TYPE: &str = "peer_delivery_acknowledged";
const PEER_MCP_SERVER: &str = "fireline-peer";
const PROMPT_PEER_TOOL: &str = "prompt_peer";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerRoutingEvent {
    pub session_id: SessionId,
    pub request_id: RequestId,
    pub tool_call_id: ToolCallId,
    pub peer_agent_name: String,
    pub prompt: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDispatchSuccess {
    pub peer_host_id: String,
    pub peer_agent_name: String,
    pub response_text: String,
    pub stop_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerDeliveryAcknowledged {
    pub session_id: SessionId,
    pub tool_call_id: ToolCallId,
    pub peer_host_id: String,
    pub peer_agent_name: String,
    pub response_text: String,
    pub stop_reason: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
}

impl PeerDeliveryAcknowledged {
    #[must_use]
    pub fn from_dispatch(event: &PeerRoutingEvent, result: PeerDispatchSuccess) -> Self {
        let meta = TraceContext::from_meta(&event.meta).into_meta();
        Self {
            session_id: event.session_id.clone(),
            tool_call_id: event.tool_call_id.clone(),
            peer_host_id: result.peer_host_id,
            peer_agent_name: result.peer_agent_name,
            response_text: result.response_text,
            stop_reason: result.stop_reason,
            meta,
        }
    }
}

#[async_trait]
pub trait PeerRoutingDispatcher: Send + Sync {
    async fn dispatch(&self, event: &PeerRoutingEvent) -> HandlerOutcome<PeerDispatchSuccess>;
}

#[derive(Clone)]
pub struct PeerRoutingSubscriber {
    dispatcher: Arc<dyn PeerRoutingDispatcher>,
}

impl PeerRoutingSubscriber {
    #[must_use]
    pub fn new(dispatcher: Arc<dyn PeerRoutingDispatcher>) -> Self {
        Self { dispatcher }
    }
}

impl DurableSubscriber for PeerRoutingSubscriber {
    type Event = PeerRoutingEvent;
    type Completion = PeerDeliveryAcknowledged;

    fn name(&self) -> &str {
        "peer_routing"
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        match_peer_routing_event(envelope)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        CompletionKey::tool(event.session_id.clone(), event.tool_call_id.clone())
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        let expected = self.completion_key(event);
        log.iter().any(|envelope| {
            envelope.entity_type == PEER_DELIVERY_ACK_ENTITY_TYPE
                && envelope.completion_key().as_ref() == Some(&expected)
        })
    }
}

#[async_trait]
impl ActiveSubscriber for PeerRoutingSubscriber {
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion> {
        match self.dispatcher.dispatch(&event).await {
            HandlerOutcome::Completed(result) => {
                HandlerOutcome::Completed(PeerDeliveryAcknowledged::from_dispatch(&event, result))
            }
            HandlerOutcome::RetryTransient(error) => HandlerOutcome::RetryTransient(error),
            HandlerOutcome::Failed(error) => HandlerOutcome::Failed(error),
        }
    }

    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChunkRowEnvelope {
    session_id: SessionId,
    request_id: RequestId,
    tool_call_id: Option<ToolCallId>,
    update: SessionUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptPeerInput {
    agent_name: String,
    prompt: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct McpToolRawInput {
    server: String,
    tool: String,
    #[serde(default)]
    params: Value,
}

fn match_peer_routing_event(envelope: &StreamEnvelope) -> Option<PeerRoutingEvent> {
    if envelope.entity_type != "chunk_v2" {
        return None;
    }

    let row: ChunkRowEnvelope = envelope.value_as()?;
    let SessionUpdate::ToolCall(tool_call) = row.update else {
        return None;
    };
    let input = prompt_peer_input(&tool_call)?;
    let tool_call_id = row
        .tool_call_id
        .unwrap_or_else(|| tool_call.tool_call_id.clone());

    Some(PeerRoutingEvent {
        session_id: row.session_id,
        request_id: row.request_id,
        tool_call_id,
        peer_agent_name: input.agent_name,
        prompt: input.prompt,
        meta: tool_call.meta.unwrap_or_default(),
    })
}

fn prompt_peer_input(tool_call: &ToolCall) -> Option<PromptPeerInput> {
    let raw_input = tool_call.raw_input.as_ref()?;

    if let Ok(input) = serde_json::from_value::<PromptPeerInput>(raw_input.clone()) {
        return Some(input);
    }

    let wrapped = serde_json::from_value::<McpToolRawInput>(raw_input.clone()).ok()?;
    if wrapped.server != PEER_MCP_SERVER || wrapped.tool != PROMPT_PEER_TOOL {
        return None;
    }
    serde_json::from_value(wrapped.params).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SuccessDispatcher;

    #[async_trait]
    impl PeerRoutingDispatcher for SuccessDispatcher {
        async fn dispatch(&self, event: &PeerRoutingEvent) -> HandlerOutcome<PeerDispatchSuccess> {
            HandlerOutcome::Completed(PeerDispatchSuccess {
                peer_host_id: "host-b".to_string(),
                peer_agent_name: event.peer_agent_name.clone(),
                response_text: format!("echo: {}", event.prompt),
                stop_reason: "end_turn".to_string(),
            })
        }
    }

    fn prompt_peer_envelope(raw_input: Value, meta: Map<String, Value>) -> StreamEnvelope {
        StreamEnvelope::from_json(serde_json::json!({
            "type": "chunk_v2",
            "key": "session-a:req-1:tool-1:0",
            "headers": { "operation": "insert" },
            "value": {
                "sessionId": "session-a",
                "requestId": "req-1",
                "toolCallId": "tool-1",
                "update": {
                    "sessionUpdate": "toolCall",
                    "toolCallId": "tool-1",
                    "title": "Prompt peer",
                    "rawInput": raw_input,
                    "_meta": meta,
                },
                "createdAt": 123
            }
        }))
        .expect("valid tool call envelope")
    }

    #[test]
    fn matches_wrapped_prompt_peer_tool_call() {
        let mut meta = Map::new();
        meta.insert(
            "traceparent".to_string(),
            Value::String("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        );
        meta.insert(
            "tracestate".to_string(),
            Value::String("vendor=value".to_string()),
        );

        let envelope = prompt_peer_envelope(
            serde_json::json!({
                "server": "fireline-peer",
                "tool": "prompt_peer",
                "params": {
                    "agentName": "agent-b",
                    "prompt": "hello across mesh"
                }
            }),
            meta.clone(),
        );

        let event = match_peer_routing_event(&envelope).expect("prompt_peer event should match");
        assert_eq!(event.session_id, SessionId::from("session-a".to_string()));
        assert_eq!(event.request_id, RequestId::Str("req-1".into()));
        assert_eq!(event.tool_call_id, ToolCallId::from("tool-1".to_string()));
        assert_eq!(event.peer_agent_name, "agent-b");
        assert_eq!(event.prompt, "hello across mesh");
        assert_eq!(event.meta, meta);
    }

    #[test]
    fn ignores_non_peer_tool_calls() {
        let envelope = prompt_peer_envelope(
            serde_json::json!({
                "server": "fireline-peer",
                "tool": "list_peers",
                "params": {}
            }),
            Map::new(),
        );

        assert!(
            match_peer_routing_event(&envelope).is_none(),
            "non-prompt_peer tool calls must not match the peer-routing subscriber"
        );
    }

    #[tokio::test]
    async fn completion_stays_caller_local_and_trace_only() {
        let subscriber = PeerRoutingSubscriber::new(Arc::new(SuccessDispatcher));
        let mut meta = Map::new();
        meta.insert(
            "traceparent".to_string(),
            Value::String("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        );
        meta.insert(
            "baggage".to_string(),
            Value::String("tenant=default".to_string()),
        );
        meta.insert("extra".to_string(), Value::String("drop-me".to_string()));

        let event = PeerRoutingEvent {
            session_id: SessionId::from("caller-session".to_string()),
            request_id: RequestId::Str("caller-request".into()),
            tool_call_id: ToolCallId::from("tool-123".to_string()),
            peer_agent_name: "agent-b".to_string(),
            prompt: "hello".to_string(),
            meta,
        };

        let HandlerOutcome::Completed(completion) = subscriber.handle(event.clone()).await else {
            panic!("peer routing should complete successfully");
        };

        assert_eq!(completion.session_id, event.session_id);
        assert_eq!(completion.tool_call_id, event.tool_call_id);
        assert_eq!(completion.peer_host_id, "host-b");
        assert_eq!(completion.peer_agent_name, "agent-b");
        assert_eq!(completion.response_text, "echo: hello");
        assert!(
            !completion.meta.contains_key("extra"),
            "completion metadata must carry only trace lineage, not ad hoc cross-session fields"
        );
        assert_eq!(
            completion.meta.get("traceparent").and_then(Value::as_str),
            Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
        );
        assert_eq!(
            completion.meta.get("baggage").and_then(Value::as_str),
            Some("tenant=default")
        );
    }

    #[test]
    fn completed_check_is_caller_local() {
        let subscriber = PeerRoutingSubscriber::new(Arc::new(SuccessDispatcher));
        let event = PeerRoutingEvent {
            session_id: SessionId::from("caller-session".to_string()),
            request_id: RequestId::Str("caller-request".into()),
            tool_call_id: ToolCallId::from("tool-123".to_string()),
            peer_agent_name: "agent-b".to_string(),
            prompt: "hello".to_string(),
            meta: Map::new(),
        };

        let matching = StreamEnvelope::from_json(serde_json::json!({
            "type": "peer_delivery_acknowledged",
            "key": "caller-session:tool-123",
            "headers": { "operation": "insert" },
            "value": {
                "sessionId": "caller-session",
                "toolCallId": "tool-123",
                "peerHostId": "host-b",
                "peerAgentName": "agent-b",
                "responseText": "ok",
                "stopReason": "end_turn"
            }
        }))
        .expect("valid matching completion envelope");
        let remote = StreamEnvelope::from_json(serde_json::json!({
            "type": "peer_delivery_acknowledged",
            "key": "child-session:tool-123",
            "headers": { "operation": "insert" },
            "value": {
                "sessionId": "child-session",
                "toolCallId": "tool-123",
                "peerHostId": "host-b",
                "peerAgentName": "agent-b",
                "responseText": "ok",
                "stopReason": "end_turn"
            }
        }))
        .expect("valid non-matching completion envelope");

        assert!(
            subscriber.is_completed(&event, &[remote.clone(), matching.clone()]),
            "matching caller-local completion must satisfy the subscriber"
        );
        assert!(
            !subscriber.is_completed(&event, &[remote]),
            "remote child session ids must not satisfy caller-local completion"
        );
    }
}
