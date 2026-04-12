#![forbid(unsafe_code)]

use std::path::PathBuf;

pub const ALLOW_LEGACY_HEADER: &str = "fireline-verify: allow-legacy-agent-identifiers";
pub const FORBIDDEN_IDENTIFIERS: &[&str] = &[
    "prompt_turn_id",
    "trace_id",
    "parent_prompt_turn_id",
    "chunk_seq",
    "chunk_id",
    "logical_connection_id",
    "edge_id",
];

pub fn strict_audit_enabled() -> bool {
    cfg!(feature = "strict-audit")
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}
