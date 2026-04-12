use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait ManagedSandbox: Send {
    async fn shutdown(self: Box<Self>) -> Result<()>;
}
