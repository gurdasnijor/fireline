//! Peer wire transport.
//!
//! How `prompt_peer` actually reaches the target peer.
//!
//! **Today (HTTP)**: dispatches via the peer's helper API (a REST
//! endpoint on the peer's binary). This is the bootstrap path that
//! works without any cross-instance ACP work.
//!
//! **After the peer-native spike**: dispatches via the peer's ACP
//! endpoint using `sacp`'s `Component<Conductor>` mechanism. The
//! outgoing `initialize` request carries `_meta.parentSessionId` and
//! `_meta.parentPromptTurnId` so the receiving Fireline can record
//! the lineage.
//!
//! Both wire variants live in this module behind a single function
//! signature so the rest of the crate doesn't care which one is in
//! use.

// TODO: implement peer transport
//
// Target signature:
//
// ```rust,ignore
// pub(crate) async fn dispatch_peer_call(
//     peer: &Peer,
//     prompt_text: &str,
//     parent_lineage: Option<ParentLineage>,
// ) -> anyhow::Result<PeerCallResult>;
//
// pub(crate) struct PeerCallResult {
//     pub response_text: String,
//     pub stop_reason: String,
// }
//
// pub(crate) struct ParentLineage {
//     pub parent_session_id: String,
//     pub parent_prompt_turn_id: String,
// }
// ```
