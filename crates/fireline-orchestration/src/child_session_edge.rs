use std::fmt::Write as _;

use async_trait::async_trait;
use durable_streams::Producer;
use fireline_tools::lookup::{ChildSessionEdgeInput, ChildSessionEdgeSink};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct ChildSessionEdgeWriter {
    producer: Producer,
}

impl ChildSessionEdgeWriter {
    pub fn new(producer: Producer) -> Self {
        Self { producer }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChildSessionEdgeRow {
    edge_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<String>,
    parent_runtime_id: String,
    parent_session_id: String,
    parent_prompt_turn_id: String,
    child_runtime_id: String,
    child_session_id: String,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
struct StateHeaders {
    operation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct StateEnvelope<T> {
    #[serde(rename = "type")]
    entity_type: &'static str,
    key: String,
    headers: StateHeaders,
    value: T,
}

#[async_trait]
impl ChildSessionEdgeSink for ChildSessionEdgeWriter {
    async fn emit_child_session_edge(&self, edge: ChildSessionEdgeInput) -> anyhow::Result<()> {
        let row = ChildSessionEdgeRow {
            edge_id: edge_id(&edge),
            trace_id: edge.trace_id,
            parent_runtime_id: edge.parent_runtime_id,
            parent_session_id: edge.parent_session_id,
            parent_prompt_turn_id: edge.parent_prompt_turn_id,
            child_runtime_id: edge.child_runtime_id,
            child_session_id: edge.child_session_id,
            created_at: now_ms(),
        };

        self.producer.append_json(&StateEnvelope {
            entity_type: "child_session_edge",
            key: row.edge_id.clone(),
            headers: StateHeaders {
                operation: "insert",
            },
            value: row,
        });

        Ok(())
    }
}

fn edge_id(edge: &ChildSessionEdgeInput) -> String {
    let mut hasher = Sha256::new();
    hasher.update(edge.parent_runtime_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(edge.parent_session_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(edge.parent_prompt_turn_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(edge.child_runtime_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(edge.child_session_id.as_bytes());

    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::edge_id;
    use fireline_tools::lookup::ChildSessionEdgeInput;

    #[test]
    fn edge_id_is_deterministic_for_same_topology() {
        let edge = ChildSessionEdgeInput {
            trace_id: Some("trace-1".to_string()),
            parent_runtime_id: "runtime-a".to_string(),
            parent_session_id: "session-a".to_string(),
            parent_prompt_turn_id: "turn-a".to_string(),
            child_runtime_id: "runtime-b".to_string(),
            child_session_id: "session-b".to_string(),
        };

        assert_eq!(edge_id(&edge), edge_id(&edge));
    }
}
