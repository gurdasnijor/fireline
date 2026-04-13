use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use fireline_harness::{ApprovalAction, ApprovalConfig, ApprovalMatch, ApprovalPolicy};
use fireline_harness::{
    AutoApproveConfig, AutoApproveSubscriber, CompletionKey, DurableSubscriber,
    DurableSubscriberDriver, HandlerOutcome, PEER_DELIVERY_ACK_ENTITY_TYPE, PassiveSubscriber,
    PeerDeliveryAcknowledged, PeerDispatchSuccess, PeerRoutingDispatcher, PeerRoutingEvent,
    PeerRoutingSubscriber, StreamEnvelope, SubscriberMode, SubscriberRegistration,
    WebhookDelivered, WebhookEventSelector, WebhookSubscriber, WebhookSubscriberConfig,
    WebhookTargetConfig, approval_resolution_envelope, permission_request_envelope,
};
use sacp::schema::{RequestId, SessionId, SessionUpdate, ToolCall, ToolCallId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Clone)]
struct ApprovalPassiveProfile;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalPermissionEvent {
    kind: String,
    session_id: SessionId,
    request_id: Option<RequestId>,
}

impl DurableSubscriber for ApprovalPassiveProfile {
    type Event = ApprovalPermissionEvent;
    type Completion = ApprovalPermissionEvent;

    fn name(&self) -> &str {
        "approval_gate"
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        let event: ApprovalPermissionEvent = envelope.value_as()?;
        (event.kind == "permission_request" && event.request_id.is_some()).then_some(event)
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        CompletionKey::prompt(
            event.session_id.clone(),
            event
                .request_id
                .clone()
                .expect("approval permission_request must carry request_id"),
        )
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
                .and_then(Value::as_str)
                .map(|text| SessionId::from(text.to_string()))
            else {
                return false;
            };
            let Some(request_id) = value
                .get("requestId")
                .and_then(|value| serde_json::from_value::<RequestId>(value.clone()).ok())
            else {
                return false;
            };
            session_id == event.session_id
                && event
                    .request_id
                    .as_ref()
                    .is_some_and(|expected| expected == &request_id)
        })
    }
}

impl PassiveSubscriber for ApprovalPassiveProfile {}

struct NoopPeerDispatcher;

#[async_trait]
impl PeerRoutingDispatcher for NoopPeerDispatcher {
    async fn dispatch(&self, event: &PeerRoutingEvent) -> HandlerOutcome<PeerDispatchSuccess> {
        panic!(
            "cross-profile registration test should not dispatch peer routing for {:?}",
            event
        );
    }
}

fn webhook_config() -> WebhookSubscriberConfig {
    WebhookSubscriberConfig {
        target: "slack-approvals".to_string(),
        events: vec![WebhookEventSelector::Kind("permission_request".to_string())],
        target_config: WebhookTargetConfig {
            url: "http://127.0.0.1/ignored".to_string(),
            headers: BTreeMap::new(),
            timeout_ms: 1_000,
            max_attempts: 3,
            cursor_stream: "subscribers:webhook:test".to_string(),
            dead_letter_stream: None,
        },
        source_stream_url: Some("http://streams/state/session-a".to_string()),
        retry_policy: None,
    }
}

fn permission_request() -> StreamEnvelope {
    permission_request_envelope(
        SessionId::from("session-a"),
        RequestId::from("req-1".to_string()),
        "approval required".to_string(),
    )
    .expect("build permission_request envelope")
}

fn approval_resolved() -> StreamEnvelope {
    approval_resolution_envelope(
        SessionId::from("session-a"),
        RequestId::from("req-1".to_string()),
        true,
        "approver".to_string(),
    )
    .expect("build approval_resolved envelope")
}

fn webhook_delivered() -> StreamEnvelope {
    StreamEnvelope::from_json(json!({
        "type": "webhook_delivery",
        "key": "slack-approvals:prompt:session-a:req-1:delivered",
        "headers": { "operation": "insert" },
        "value": WebhookDelivered {
            kind: "webhook_delivered".to_string(),
            target: "slack-approvals".to_string(),
            session_id: SessionId::from("session-a"),
            request_id: Some(RequestId::from("req-1".to_string())),
            tool_call_id: None,
            offset: "0000000000000001_0000000000000000".to_string(),
            delivered_at_ms: 123,
            status_code: Some(200),
            meta: None,
        }
    }))
    .expect("build webhook_delivered envelope")
}

fn peer_prompt_event() -> StreamEnvelope {
    let mut meta = Map::new();
    meta.insert(
        "traceparent".to_string(),
        Value::String("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
    );
    let update = serde_json::to_value(SessionUpdate::ToolCall(
        ToolCall::new("tool-1", "Prompt peer")
            .raw_input(json!({
                "server": "fireline-peer",
                "tool": "prompt_peer",
                "params": {
                    "agentName": "agent-b",
                    "prompt": "hello across mesh",
                }
            }))
            .meta(meta),
    ))
    .expect("serialize tool call update");

    StreamEnvelope::from_json(json!({
        "type": "chunk_v2",
        "key": "session-a:req-1:tool-1:0",
        "headers": { "operation": "insert" },
        "value": {
            "sessionId": "session-a",
            "requestId": "req-1",
            "toolCallId": "tool-1",
            "update": update,
            "createdAt": 123
        }
    }))
    .expect("build peer prompt event")
}

fn peer_delivery_ack() -> StreamEnvelope {
    StreamEnvelope::from_json(json!({
        "type": PEER_DELIVERY_ACK_ENTITY_TYPE,
        "key": "session-a:tool-1",
        "headers": { "operation": "insert" },
        "value": PeerDeliveryAcknowledged {
            session_id: SessionId::from("session-a"),
            tool_call_id: ToolCallId::from("tool-1".to_string()),
            peer_host_id: "host-b".to_string(),
            peer_agent_name: "agent-b".to_string(),
            response_text: "echo: hello across mesh".to_string(),
            stop_reason: "end_turn".to_string(),
            meta: Map::new(),
        }
    }))
    .expect("build peer delivery ack envelope")
}

#[test]
fn driver_registers_cross_profile_inventory_without_mode_collisions() {
    let approval = ApprovalPassiveProfile;
    let auto_approve = AutoApproveSubscriber::new(AutoApproveConfig::default());
    let webhook = WebhookSubscriber::new(webhook_config());
    let peer = PeerRoutingSubscriber::new(Arc::new(NoopPeerDispatcher));

    let mut driver = DurableSubscriberDriver::new();
    driver
        .register_passive(approval)
        .register_active(auto_approve)
        .register_active(webhook)
        .register_active(peer);

    assert_eq!(
        driver.registrations(),
        vec![
            SubscriberRegistration {
                name: "approval_gate".to_string(),
                mode: SubscriberMode::Passive,
            },
            SubscriberRegistration {
                name: "auto_approve".to_string(),
                mode: SubscriberMode::Active,
            },
            SubscriberRegistration {
                name: "webhook_subscriber".to_string(),
                mode: SubscriberMode::Active,
            },
            SubscriberRegistration {
                name: "peer_routing".to_string(),
                mode: SubscriberMode::Active,
            },
        ],
        "cross-profile driver inventory should preserve each subscriber's mode and name without registration collisions",
    );
}

#[test]
fn completion_keys_and_completion_kinds_stay_profile_local_across_profiles() {
    let approval = ApprovalPassiveProfile;
    let auto_approve = AutoApproveSubscriber::new(AutoApproveConfig::default());
    let webhook = WebhookSubscriber::new(webhook_config());
    let peer = PeerRoutingSubscriber::new(Arc::new(NoopPeerDispatcher));

    let permission = permission_request();
    let peer_prompt = peer_prompt_event();

    let approval_event = approval
        .matches(&permission)
        .expect("approval passive should match permission_request");
    let auto_approve_event = auto_approve
        .matches(&permission)
        .expect("auto-approve should match permission_request");
    let webhook_event = webhook
        .matches(&permission)
        .expect("webhook should match permission_request when configured for that kind");
    let peer_event = peer
        .matches(&peer_prompt)
        .expect("peer routing should match prompt_peer tool calls");

    assert!(
        approval.matches(&peer_prompt).is_none()
            && auto_approve.matches(&peer_prompt).is_none()
            && webhook.matches(&peer_prompt).is_none(),
        "tool-scoped peer events must not leak into prompt-scoped approval or webhook profiles",
    );
    assert!(
        peer.matches(&permission).is_none(),
        "prompt-scoped permission events must not leak into peer routing",
    );

    assert_eq!(
        approval.completion_key(&approval_event),
        auto_approve.completion_key(&auto_approve_event),
        "approval passive and auto-approve must share the same prompt-scoped completion spine",
    );
    assert_eq!(
        approval.completion_key(&approval_event),
        webhook.completion_key(&webhook_event),
        "webhook deliveries reuse the same prompt-scoped canonical key while keeping their own completion kind",
    );
    assert_eq!(
        peer.completion_key(&peer_event),
        CompletionKey::tool(
            SessionId::from("session-a"),
            ToolCallId::from("tool-1".to_string()),
        ),
        "peer routing stays on the tool-scoped canonical key family",
    );

    let approval_log = vec![approval_resolved()];
    assert!(approval.is_completed(&approval_event, &approval_log));
    assert!(auto_approve.is_completed(&auto_approve_event, &approval_log));
    assert!(
        !webhook.is_completed(&webhook_event, &approval_log)
            && !peer.is_completed(&peer_event, &approval_log),
        "approval_resolved must not satisfy webhook or peer delivery completions",
    );

    let webhook_log = vec![webhook_delivered()];
    assert!(webhook.is_completed(&webhook_event, &webhook_log));
    assert!(
        !approval.is_completed(&approval_event, &webhook_log)
            && !auto_approve.is_completed(&auto_approve_event, &webhook_log)
            && !peer.is_completed(&peer_event, &webhook_log),
        "webhook_delivered must stay local to the webhook subscriber even though it shares the prompt key family",
    );

    let peer_log = vec![peer_delivery_ack()];
    assert!(peer.is_completed(&peer_event, &peer_log));
    assert!(
        !approval.is_completed(&approval_event, &peer_log)
            && !auto_approve.is_completed(&auto_approve_event, &peer_log)
            && !webhook.is_completed(&webhook_event, &peer_log),
        "peer delivery acknowledgments must stay tool-local and must not satisfy prompt-level subscribers",
    );
}

#[test]
fn approval_policy_fixture_stays_prompt_scoped_for_cross_profile_tests() {
    let config = ApprovalConfig {
        policies: vec![ApprovalPolicy {
            match_rule: ApprovalMatch::PromptContains {
                needle: "approval required".to_string(),
            },
            action: ApprovalAction::RequireApproval,
            reason: "cross-profile fixture".to_string(),
        }],
    };

    assert!(
        config
            .policy_for_prompt("approval required for webhook cross-profile test")
            .is_some(),
        "the cross-profile permission_request fixture should stay aligned with the prompt-scoped approval contract",
    );
}
