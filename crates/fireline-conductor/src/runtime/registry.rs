use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use super::provider::RuntimeDescriptor;

#[derive(Clone, Debug)]
pub struct RuntimeRegistry {
    path: PathBuf,
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
        Ok(Self { path })
    }

    pub fn default_path() -> Result<PathBuf> {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow!("resolve local data directory"))?;
        Ok(base.join("fireline").join("runtimes.toml"))
    }

    pub fn list(&self) -> Result<Vec<RuntimeDescriptor>> {
        read_runtimes(&self.path)
    }

    pub fn upsert(&self, descriptor: RuntimeDescriptor) -> Result<()> {
        let mut runtimes = self.list()?;
        runtimes.retain(|existing| existing.runtime_key != descriptor.runtime_key);
        runtimes.push(descriptor);
        write_runtimes(&self.path, &runtimes)
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
        write_runtimes(&self.path, &runtimes)?;
        Ok(removed)
    }
}

fn read_runtimes(path: &Path) -> Result<Vec<RuntimeDescriptor>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: RuntimeRegistryFile =
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(file.runtimes)
}

fn write_runtimes(path: &Path, runtimes: &[RuntimeDescriptor]) -> Result<()> {
    let raw = toml::to_string(&RuntimeRegistryFile {
        runtimes: runtimes.to_vec(),
    })
    .context("serialize runtimes.toml")?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, raw).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct RuntimeRegistryFile {
    #[serde(default)]
    runtimes: Vec<RuntimeDescriptor>,
}
