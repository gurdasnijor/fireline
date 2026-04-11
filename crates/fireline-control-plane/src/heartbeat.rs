use anyhow::Result;
use fireline_runtime::RuntimeRegistry;

#[derive(Clone)]
pub struct HeartbeatTracker {
    registry: RuntimeRegistry,
}

impl HeartbeatTracker {
    pub fn new(registry: RuntimeRegistry) -> Self {
        Self { registry }
    }

    pub async fn record(&self, runtime_key: impl Into<String>, seen_at_ms: i64) -> Result<()> {
        self.registry.record_liveness(runtime_key, seen_at_ms);
        Ok(())
    }

    pub async fn forget(&self, runtime_key: &str) -> Result<()> {
        self.registry.forget_liveness(runtime_key);
        Ok(())
    }

    pub async fn stale_keys(&self, stale_before_ms: i64) -> Result<Vec<String>> {
        Ok(self.registry.stale_liveness_keys(stale_before_ms))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use fireline_runtime::RuntimeRegistry;

    use super::HeartbeatTracker;

    #[tokio::test]
    async fn returns_only_stale_keys() -> Result<()> {
        let registry = RuntimeRegistry::load(
            std::env::temp_dir().join(format!("fireline-heartbeat-{}.toml", uuid::Uuid::new_v4())),
        )?;
        let tracker = HeartbeatTracker::new(registry);
        tracker.record("runtime:a", 100).await?;
        tracker.record("runtime:b", 250).await?;

        let stale = tracker.stale_keys(150).await?;
        assert_eq!(stale, vec!["runtime:a".to_string()]);
        Ok(())
    }

    #[tokio::test]
    async fn forget_removes_runtime() -> Result<()> {
        let registry = RuntimeRegistry::load(
            std::env::temp_dir().join(format!("fireline-heartbeat-{}.toml", uuid::Uuid::new_v4())),
        )?;
        let tracker = HeartbeatTracker::new(registry);
        tracker.record("runtime:a", 100).await?;
        tracker.forget("runtime:a").await?;

        assert!(tracker.stale_keys(200).await?.is_empty());
        Ok(())
    }
}
