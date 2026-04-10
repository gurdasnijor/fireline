use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct HeartbeatTracker {
    inner: Arc<Mutex<HashMap<String, i64>>>,
}

impl HeartbeatTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn record(&self, runtime_key: impl Into<String>, seen_at_ms: i64) {
        self.inner
            .lock()
            .await
            .insert(runtime_key.into(), seen_at_ms);
    }

    pub async fn forget(&self, runtime_key: &str) {
        self.inner.lock().await.remove(runtime_key);
    }

    pub async fn stale_keys(&self, stale_before_ms: i64) -> Vec<String> {
        self.inner
            .lock()
            .await
            .iter()
            .filter_map(|(runtime_key, seen_at_ms)| {
                if *seen_at_ms <= stale_before_ms {
                    Some(runtime_key.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::HeartbeatTracker;

    #[tokio::test]
    async fn returns_only_stale_keys() {
        let tracker = HeartbeatTracker::new();
        tracker.record("runtime:a", 100).await;
        tracker.record("runtime:b", 250).await;

        let stale = tracker.stale_keys(150).await;
        assert_eq!(stale, vec!["runtime:a".to_string()]);
    }

    #[tokio::test]
    async fn forget_removes_runtime() {
        let tracker = HeartbeatTracker::new();
        tracker.record("runtime:a", 100).await;
        tracker.forget("runtime:a").await;

        assert!(tracker.stale_keys(200).await.is_empty());
    }
}
