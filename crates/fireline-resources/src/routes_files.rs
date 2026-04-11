//! Filesystem helper API.
//!
//! REST endpoints for browsing files in agent workspaces. Reads
//! connection metadata (specifically `cwd`) from the connection
//! lookup files written by [`crate::connections`] at session
//! creation time.
//!
//! Routes:
//! - `GET /api/v1/files/{connection_id}` — read a file from the
//!   connection's workspace
//! - `GET /api/v1/files/{connection_id}/tree` — list a directory in
//!   the connection's workspace
//!
//! These endpoints exist as a practical affordance for browser UIs
//! that want to display agent workspace contents. They could
//! eventually be replaced by an MCP filesystem tool exposed via a
//! `FilesystemComponent` (mirroring how `PeerComponent` injects the
//! peer-call MCP server), but for now REST is simpler.

// TODO: implement filesystem helper API
//
// Target shape:
//
// ```rust,ignore
// pub fn router(app: AppState) -> axum::Router {
//     axum::Router::new()
//         .route("/files/{connection_id}", axum::routing::get(get_file))
//         .route("/files/{connection_id}/tree", axum::routing::get(get_tree))
//         .with_state(app)
// }
// ```
