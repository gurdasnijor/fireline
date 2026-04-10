//! Outbound webhook subscriber.
//!
//! Subscribes to the durable stream via `durable-streams` client-rust
//! and dispatches HTTP webhooks on configured state transitions. This
//! is a state-derived sink, not a server — it's the thing that
//! _consumes_ the stream and pushes outbound HTTP, opposite direction
//! from the ACP host.
//!
//! Implementation pattern: use `stream.read().offset(...).live(LiveMode::Auto).build()`
//! to get a chunk iterator, parse each chunk as a JSON record, diff
//! against a tiny in-process state map to detect transitions, and
//! dispatch webhook HTTP requests on transitions.
//!
//! ~80 lines when implemented. No shared state with the conductor;
//! just reads the stream like any other consumer.

// TODO: implement webhook subscriber
//
// Target shape:
//
// ```rust,ignore
// use durable_streams::{Client, LiveMode, Offset};
//
// pub struct WebhookConfig {
//     pub url: String,
//     pub events: Vec<String>,  // event types to deliver
// }
//
// pub async fn run_webhook_forwarder(
//     state_stream_url: String,
//     webhooks: Vec<WebhookConfig>,
// ) -> anyhow::Result<()> {
//     let client = Client::new();
//     let stream = client.stream(&state_stream_url);
//     let mut reader = stream.read()
//         .offset(Offset::Beginning)
//         .live(LiveMode::Auto)
//         .build();
//
//     while let Some(chunk) = reader.next_chunk().await? {
//         let record: serde_json::Value = serde_json::from_slice(&chunk.data)?;
//         // detect transitions, dispatch webhooks
//     }
//     Ok(())
// }
// ```
