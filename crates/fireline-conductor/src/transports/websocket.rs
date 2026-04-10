//! WebSocket transport adapter.
//!
//! Wraps an [`axum::extract::ws::WebSocket`] in a [`sacp::ByteStreams`]
//! and runs the conductor over it. Used by the binary's `/acp` route
//! handler — when a browser client opens a WebSocket to `/acp`, the
//! handler builds a fresh conductor and hands it to this adapter.
//!
//! Each WebSocket connection gets its own conductor instance with its
//! own component chain. The conductor's lifetime is bounded by the
//! WebSocket's lifetime.

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use sacp::Lines;
use sacp_conductor::ConductorImpl;

pub async fn handle_upgrade(
    conductor: ConductorImpl<sacp::Agent>,
    socket: WebSocket,
) -> Result<()> {
    let (write, read) = socket.split();

    let outgoing = SinkExt::with(
        SinkExt::sink_map_err(write, std::io::Error::other),
        |line: String| async move { Ok::<_, std::io::Error>(Message::Text(line.into())) },
    );

    let incoming = StreamExt::filter_map(read, |message| async move {
        match message {
            Ok(Message::Text(text)) => {
                let line = text.trim().to_string();
                if line.is_empty() {
                    None
                } else {
                    Some(Ok(line))
                }
            }
            Ok(Message::Binary(bytes)) => String::from_utf8(bytes.to_vec()).ok().and_then(|text| {
                let line = text.trim().to_string();
                if line.is_empty() {
                    None
                } else {
                    Some(Ok(line))
                }
            }),
            Ok(Message::Close(_)) => None,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => None,
            Err(err) => Some(Err(std::io::Error::other(err))),
        }
    });

    sacp::ConnectTo::<sacp::Agent>::connect_to(Lines::new(outgoing, incoming), conductor).await?;
    Ok(())
}
