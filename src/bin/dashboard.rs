//! Fireline dashboard binary — TUI that subscribes to a durable
//! stream and renders agent activity.
//!
//! Uses `durable-streams` client-rust directly to subscribe to the
//! state stream and applies events to a small in-process display
//! buffer (~50 lines of state) that the TUI renders from. No shared
//! state with the conductor; just a stream consumer.
//!
//! For now this is a debug/development tool. A real production
//! dashboard would more likely live in TypeScript using
//! `@fireline/client` + `useLiveQuery`.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // TODO: implement dashboard TUI
    //
    // Target shape:
    //
    // ```rust,ignore
    // let client = durable_streams::Client::new();
    // let stream = client.stream(&stream_url);
    // let mut reader = stream.read()
    //     .offset(durable_streams::Offset::Beginning)
    //     .live(durable_streams::LiveMode::Auto)
    //     .build();
    //
    // let mut display_state = DisplayState::new();
    // while let Some(chunk) = reader.next_chunk().await? {
    //     let event: serde_json::Value = serde_json::from_slice(&chunk.data)?;
    //     display_state.apply(&event);
    //     render_tui(&display_state)?;
    // }
    // ```

    Ok(())
}
