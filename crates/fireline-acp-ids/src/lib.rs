//! Canonical ACP identifier types for the agent plane.
//!
//! These are thin re-exports of `sacp::schema` types. Fireline does NOT invent
//! its own agent-identity types. See
//! `docs/proposals/acp-canonical-identifiers.md`.
//!
//! ToolCallId note: repository scan on 2026-04-12 found no current Fireline
//! tool-execution seam consuming canonical `ToolCallId` directly, so a future
//! upstream ACP issue may still be needed for that exposure. Phase 1 is
//! additive only and does not block on that gap.

pub use sacp::schema::{RequestId, SessionId, ToolCallId};

/// Composite reference to an ACP prompt request.
///
/// Composed only of canonical ACP identifiers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PromptRequestRef {
    pub session_id: SessionId,
    pub request_id: RequestId,
}

/// Composite reference to an ACP tool invocation within a session.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ToolInvocationRef {
    pub session_id: SessionId,
    pub tool_call_id: ToolCallId,
}

#[cfg(test)]
mod tests {
    use super::{PromptRequestRef, RequestId, SessionId};

    #[test]
    fn prompt_request_ref_round_trips_through_serde() {
        let prompt = PromptRequestRef {
            session_id: SessionId::from("test-session"),
            request_id: RequestId::from("request-1".to_string()),
        };

        let encoded = serde_json::to_string(&prompt).expect("serialize prompt request ref");
        let decoded: PromptRequestRef =
            serde_json::from_str(&encoded).expect("deserialize prompt request ref");

        assert_eq!(decoded, prompt);
    }
}
