use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::provider::RuntimeDescriptor;

#[derive(Clone, Debug)]
pub struct RuntimeRegistry {
    path: PathBuf,
    liveness: Arc<Mutex<HashMap<String, i64>>>,
}

impl RuntimeRegistry {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create runtime registry parent {}", parent.display()))?;
        }
        if !path.exists() {
            fs::write(&path, "").with_context(|| format!("initialize {}", path.display()))?;
        }
        Ok(Self {
            path,
            liveness: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn default_path() -> Result<PathBuf> {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow!("resolve local data directory"))?;
        Ok(base.join("fireline").join("runtimes.toml"))
    }

    pub fn list(&self) -> Result<Vec<RuntimeDescriptor>> {
        Ok(read_registry_file(&self.path)?.runtimes)
    }

    pub fn upsert(&self, descriptor: RuntimeDescriptor) -> Result<()> {
        let mut file = read_registry_file(&self.path)?;
        file.runtimes
            .retain(|existing| existing.runtime_key != descriptor.runtime_key);
        file.runtimes.push(descriptor);
        write_registry_file(&self.path, &file)
    }

    pub fn get(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|runtime| runtime.runtime_key == runtime_key))
    }

    pub fn remove(&self, runtime_key: &str) -> Result<Option<RuntimeDescriptor>> {
        let mut runtimes = self.list()?;
        let removed = runtimes
            .iter()
            .find(|runtime| runtime.runtime_key == runtime_key)
            .cloned();
        runtimes.retain(|runtime| runtime.runtime_key != runtime_key);
        self.forget_liveness(runtime_key);
        write_runtimes(&self.path, &runtimes)?;
        Ok(removed)
    }

    pub fn record_liveness(&self, runtime_key: impl Into<String>, seen_at_ms: i64) {
        self.liveness
            .lock()
            .expect("runtime liveness lock poisoned")
            .insert(runtime_key.into(), seen_at_ms);
    }

    pub fn forget_liveness(&self, runtime_key: &str) {
        self.liveness
            .lock()
            .expect("runtime liveness lock poisoned")
            .remove(runtime_key);
    }

    pub fn stale_liveness_keys(&self, stale_before_ms: i64) -> Vec<String> {
        self.liveness
            .lock()
            .expect("runtime liveness lock poisoned")
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

fn read_registry_file(path: &Path) -> Result<RuntimeRegistryFile> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(RuntimeRegistryFile::default());
    }
    toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn write_registry_file(path: &Path, file: &RuntimeRegistryFile) -> Result<()> {
    let raw = toml::to_string(file).context("serialize runtimes.toml")?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, raw).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn write_runtimes(path: &Path, runtimes: &[RuntimeDescriptor]) -> Result<()> {
    write_registry_file(
        path,
        &RuntimeRegistryFile {
            runtimes: runtimes.to_vec(),
        },
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RuntimeRegistryFile {
    #[serde(default)]
    runtimes: Vec<RuntimeDescriptor>,
}

#[cfg(test)]
mod tests {
    use super::{RuntimeRegistry, RuntimeRegistryFile};
    use crate::{Endpoint, RuntimeDescriptor, RuntimeProviderKind, RuntimeStatus};
    use anyhow::Result;

    #[test]
    fn liveness_round_trips_without_changing_runtime_descriptors() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "fireline-runtime-registry-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let registry = RuntimeRegistry::load(&path)?;
        registry.upsert(RuntimeDescriptor {
            runtime_key: "runtime:test".to_string(),
            runtime_id: "runtime-id".to_string(),
            node_id: "node:test".to_string(),
            provider: RuntimeProviderKind::Local,
            provider_instance_id: "instance:test".to_string(),
            status: RuntimeStatus::Ready,
            acp: Endpoint::new("ws://127.0.0.1:4444/acp"),
            state: Endpoint::new("http://127.0.0.1:4444/v1/stream/fireline"),
            helper_api_base_url: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        })?;

        registry.record_liveness("runtime:test", 123);

        let listed = registry.list()?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].runtime_key, "runtime:test");
        assert_eq!(registry.stale_liveness_keys(122), Vec::<String>::new());
        assert_eq!(
            registry.stale_liveness_keys(123),
            vec!["runtime:test".to_string()]
        );

        let raw = std::fs::read_to_string(&path)?;
        let file: RuntimeRegistryFile = toml::from_str(&raw)?;
        assert_eq!(file.runtimes.len(), 1);

        Ok(())
    }

    #[test]
    fn removing_runtime_forgets_liveness() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "fireline-runtime-registry-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let registry = RuntimeRegistry::load(&path)?;
        registry.record_liveness("runtime:test", 123);
        registry.remove("runtime:test")?;
        assert!(registry.stale_liveness_keys(123).is_empty());
        Ok(())
    }
}
