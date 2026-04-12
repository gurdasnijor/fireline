use anyhow::{Context, Result};
use axum::Router;
use tokio::sync::oneshot;

pub(crate) struct TestStreamServer {
    pub(crate) base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestStreamServer {
    pub(crate) async fn spawn() -> Result<Self> {
        let router: Router = fireline_session::build_stream_router(None)?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("bind durable-streams test listener")?;
        let addr = listener
            .local_addr()
            .context("resolve durable-streams test listener")?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Ok(Self {
            base_url: format!("http://127.0.0.1:{}/v1/stream", addr.port()),
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    pub(crate) fn stream_url(&self, stream_name: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), stream_name)
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}
