//! Process bootstrap.
//!
//! `bootstrap::start(config)` brings up everything the Fireline
//! binary needs in one process:
//!
//! 1. Build the embedded durable-streams Router via
//!    [`crate::stream_host::build_stream_router`]
//! 2. Build the `durable-streams::Producer` that the trace writer
//!    will append to (HTTP client pointed at our own listener)
//! 3. Compose the component list (`PeerComponent`, future components)
//! 4. Build the axum Router and `.merge()` in the stream Router so
//!    `/healthz`, `/v1/stream/{name}`, `/acp`, and `/api/v1/files/*`
//!    all live on a single listener (Option A embedding)
//! 5. Spawn the webhook subscriber if any webhooks are configured
//! 6. Bind the listener on `config.host:config.port` and serve
//! 7. Return a handle that can be `.shutdown()`'d gracefully

// TODO: implement bootstrap::start
//
// Target shape:
//
// ```rust,ignore
// pub struct BootstrapConfig {
//     pub host: std::net::IpAddr,
//     pub port: u16,
//     pub name: String,
//     pub agent_command: Vec<String>,
// }
//
// pub struct BootstrapHandle {
//     // axum server handle, webhook task handles
// }
//
// pub async fn start(config: BootstrapConfig) -> anyhow::Result<BootstrapHandle> {
//     let stream_router = crate::stream_host::build_stream_router()?;
//
//     // ... build AppState (producer, peer directory, runtime id, agent cmd) ...
//
//     let app = axum::Router::new()
//         .merge(crate::routes::acp::router(app_state.clone()))
//         .nest("/api/v1/files", crate::routes::files::router(app_state.clone()))
//         .merge(stream_router);
//
//     let addr = std::net::SocketAddr::new(config.host, config.port);
//     let listener = tokio::net::TcpListener::bind(addr).await?;
//     // ... axum::serve with graceful shutdown ...
// }
//
// impl BootstrapHandle {
//     pub async fn shutdown(self) -> anyhow::Result<()>;
// }
// ```
