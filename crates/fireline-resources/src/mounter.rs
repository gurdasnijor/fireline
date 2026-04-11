use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::provider::ResourceRef;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MountedResource {
    pub host_path: PathBuf,
    pub mount_path: PathBuf,
    pub read_only: bool,
}

#[async_trait]
pub trait ResourceMounter: Send + Sync {
    async fn mount(
        &self,
        resource: &ResourceRef,
        runtime_key: &str,
    ) -> Result<Option<MountedResource>>;
}

#[derive(Debug, Clone, Default)]
pub struct LocalPathMounter;

impl LocalPathMounter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ResourceMounter for LocalPathMounter {
    async fn mount(
        &self,
        resource: &ResourceRef,
        _runtime_key: &str,
    ) -> Result<Option<MountedResource>> {
        let ResourceRef::LocalPath { path, mount_path } = resource else {
            return Ok(None);
        };

        if !mount_path.is_absolute() {
            return Err(anyhow!(
                "resource mount path '{}' must be absolute",
                mount_path.display()
            ));
        }

        let host_path = std::fs::canonicalize(path)
            .with_context(|| format!("resolve local resource path {}", path.display()))?;
        Ok(Some(MountedResource {
            host_path,
            mount_path: mount_path.clone(),
            read_only: true,
        }))
    }
}

pub async fn prepare_resources(
    resources: &[ResourceRef],
    mounters: &[Arc<dyn ResourceMounter>],
    runtime_key: &str,
) -> Result<Vec<MountedResource>> {
    let mut mounted = Vec::with_capacity(resources.len());
    for resource in resources {
        let mut prepared = None;
        for mounter in mounters {
            if let Some(candidate) = mounter.mount(resource, runtime_key).await? {
                prepared = Some(candidate);
                break;
            }
        }

        let Some(prepared) = prepared else {
            return Err(anyhow!(
                "no resource mounter configured for {}",
                resource_label(resource)
            ));
        };
        mounted.push(prepared);
    }

    Ok(mounted)
}

fn resource_label(resource: &ResourceRef) -> &'static str {
    match resource {
        ResourceRef::LocalPath { .. } => "local_path",
        ResourceRef::GitRemote { .. } => "git_remote",
        ResourceRef::S3 { .. } => "s3",
        ResourceRef::Gcs { .. } => "gcs",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{LocalPathMounter, ResourceMounter};
    use crate::runtime::ResourceRef;

    #[tokio::test]
    async fn local_path_mounter_canonicalizes_host_path() {
        let root = std::env::temp_dir().join(format!(
            "fireline-local-path-mounter-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("hello.txt");
        std::fs::write(&file, "hello").unwrap();

        let mounted = LocalPathMounter::new()
            .mount(
                &ResourceRef::LocalPath {
                    path: root.join(".").join("hello.txt"),
                    mount_path: PathBuf::from("/work/hello.txt"),
                },
                "runtime:test",
            )
            .await
            .unwrap()
            .expect("resource should mount");

        assert_eq!(mounted.host_path, std::fs::canonicalize(&file).unwrap());
        assert_eq!(mounted.mount_path, PathBuf::from("/work/hello.txt"));
        assert!(mounted.read_only);

        let _ = std::fs::remove_dir_all(root);
    }
}
