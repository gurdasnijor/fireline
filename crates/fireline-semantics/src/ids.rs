//! Canonical ACP identifier types for the agent plane.
//!
//! These are thin re-exports of `sacp::schema` types. Fireline does NOT invent
//! its own agent-identity types. See `docs/proposals/acp-canonical-identifiers.md`.
//!
//! ToolCallId note: repository scan on 2026-04-12 found no current Fireline
//! tool-execution seam consuming canonical `ToolCallId` directly, so a future
//! upstream ACP issue may still be needed for that seam. Phase 1 is additive
//! only and does not block on that gap.

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
    use super::{PromptRequestRef, RequestId, SessionId, ToolCallId, ToolInvocationRef};

    #[test]
    fn canonical_id_types_are_reachable() {
        let session_id: SessionId = "test-session".into();
        let request_id: RequestId = "request-1".to_string().into();
        let tool_call_id: ToolCallId = "tool-call-1".into();

        let prompt = PromptRequestRef {
            session_id: session_id.clone(),
            request_id: request_id.clone(),
        };
        let tool = ToolInvocationRef {
            session_id: session_id.clone(),
            tool_call_id: tool_call_id.clone(),
        };

        let encoded = serde_json::to_string(&prompt).expect("serialize prompt request ref");
        let decoded: PromptRequestRef =
            serde_json::from_str(&encoded).expect("deserialize prompt request ref");

        assert_eq!(decoded.session_id, session_id);
        assert_eq!(decoded.request_id, request_id);
        assert_eq!(tool.tool_call_id, tool_call_id);
    }
}
