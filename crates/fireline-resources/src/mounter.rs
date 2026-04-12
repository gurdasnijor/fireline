use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset};
use serde::{Deserialize, Serialize};

use crate::{ResourceRef, ResourceSourceRef};

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
        host_key: &str,
    ) -> Result<Option<MountedResource>>;
}

#[derive(Debug, Clone, Default)]
pub struct LocalPathMounter;

impl LocalPathMounter {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone)]
pub struct DurableStreamMounter {
    client: DurableStreamsClient,
}

impl DurableStreamMounter {
    pub fn new(client: DurableStreamsClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ResourceMounter for LocalPathMounter {
    async fn mount(
        &self,
        resource: &ResourceRef,
        _host_key: &str,
    ) -> Result<Option<MountedResource>> {
        let ResourceSourceRef::LocalPath { path, .. } = &resource.source_ref else {
            return Ok(None);
        };
        let mount_path = &resource.mount_path;

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
            read_only: resource.read_only,
        }))
    }
}

#[async_trait]
impl ResourceMounter for DurableStreamMounter {
    async fn mount(
        &self,
        resource: &ResourceRef,
        host_key: &str,
    ) -> Result<Option<MountedResource>> {
        let ResourceSourceRef::DurableStreamBlob { stream, key } = &resource.source_ref else {
            return Ok(None);
        };
        let mount_path = &resource.mount_path;

        if !mount_path.is_absolute() {
            return Err(anyhow!(
                "resource mount path '{}' must be absolute",
                mount_path.display()
            ));
        }

        let host_path = materialized_blob_path(host_key, stream, key)?;
        if let Some(parent) = host_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("create durable stream blob parent {}", parent.display())
            })?;
        }

        let bytes = read_blob_bytes(&self.client, stream)
            .await
            .with_context(|| format!("read durable stream blob from stream '{stream}'"))?;
        tokio::fs::write(&host_path, bytes)
            .await
            .with_context(|| format!("write durable stream blob to {}", host_path.display()))?;

        Ok(Some(MountedResource {
            host_path,
            mount_path: mount_path.clone(),
            read_only: resource.read_only,
        }))
    }
}

pub async fn prepare_resources(
    resources: &[ResourceRef],
    mounters: &[Arc<dyn ResourceMounter>],
    host_key: &str,
) -> Result<Vec<MountedResource>> {
    let mut mounted = Vec::with_capacity(resources.len());
    for resource in resources {
        let mut prepared = None;
        for mounter in mounters {
            if let Some(candidate) = mounter.mount(resource, host_key).await? {
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
    match &resource.source_ref {
        ResourceSourceRef::LocalPath { .. } => "local_path",
        ResourceSourceRef::S3 { .. } => "s3",
        ResourceSourceRef::Gcs { .. } => "gcs",
        ResourceSourceRef::DockerVolume { .. } => "docker_volume",
        ResourceSourceRef::DurableStreamBlob { .. } => "durable_stream_blob",
        ResourceSourceRef::StreamFs { .. } => "stream_fs",
        ResourceSourceRef::OciImageLayer { .. } => "oci_image_layer",
        ResourceSourceRef::GitRepo { .. } => "git_repo",
        ResourceSourceRef::HttpUrl { .. } => "http_url",
    }
}

async fn read_blob_bytes(client: &DurableStreamsClient, stream_url: &str) -> Result<Vec<u8>> {
    let stream = client.stream(stream_url);
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Off)
        .build()
        .with_context(|| format!("build durable stream reader for '{stream_url}'"))?;

    let mut bytes = Vec::new();
    while let Some(chunk) = reader
        .next_chunk()
        .await
        .with_context(|| format!("read durable stream chunk from '{stream_url}'"))?
    {
        bytes.extend_from_slice(&chunk.data);
        if chunk.up_to_date {
            break;
        }
    }

    Ok(bytes)
}

fn materialized_blob_path(host_key: &str, stream: &str, key: &str) -> Result<PathBuf> {
    Ok(Path::new("/tmp/fireline-mounts")
        .join(sanitize_path_component(host_key))
        .join(sanitize_path_component(stream))
        .join(normalize_blob_key(key)?))
}

fn normalize_blob_key(key: &str) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    let mut saw_component = false;

    for component in Path::new(key).components() {
        match component {
            Component::Normal(segment) => {
                normalized.push(segment);
                saw_component = true;
            }
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir => {
                return Err(anyhow!(
                    "durable stream blob key '{key}' must not contain parent directory traversal"
                ));
            }
            Component::Prefix(_) => {
                return Err(anyhow!(
                    "durable stream blob key '{key}' must be a relative path"
                ));
            }
        }
    }

    if !saw_component {
        normalized.push("blob");
    }

    Ok(normalized)
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
            _ => '_',
        })
        .collect();

    if sanitized.is_empty() {
        "blob".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use durable_streams::Client as DurableStreamsClient;

    use super::{DurableStreamMounter, LocalPathMounter, ResourceMounter, materialized_blob_path};
    use crate::{ResourceRef, ResourceSourceRef};

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
                &ResourceRef {
                    source_ref: ResourceSourceRef::LocalPath {
                        host_id: "host-local".to_string(),
                        path: root.join(".").join("hello.txt"),
                    },
                    mount_path: PathBuf::from("/work/hello.txt"),
                    read_only: true,
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

    #[test]
    fn durable_stream_blob_path_scopes_materialization_to_runtime_and_stream() {
        let path = materialized_blob_path(
            "runtime:test/1",
            "https://streams.example.test/resources/demo",
            "nested/blob.txt",
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from(
                "/tmp/fireline-mounts/runtime_test_1/https___streams.example.test_resources_demo/nested/blob.txt"
            )
        );
    }

    #[test]
    fn durable_stream_blob_path_uses_blob_name_for_root_key() {
        let path = materialized_blob_path("runtime:test", "resources:demo", "/").unwrap();

        assert_eq!(
            path,
            PathBuf::from("/tmp/fireline-mounts/runtime_test/resources_demo/blob")
        );
    }

    #[test]
    fn durable_stream_blob_path_rejects_parent_traversal() {
        let error = materialized_blob_path("runtime:test", "resources:demo", "../secret.txt")
            .expect_err("parent traversal should fail");

        assert!(
            error
                .to_string()
                .contains("must not contain parent directory traversal")
        );
    }

    #[tokio::test]
    async fn durable_stream_mounter_ignores_non_blob_resources() {
        let mounted = DurableStreamMounter::new(DurableStreamsClient::new())
            .mount(
                &ResourceRef {
                    source_ref: ResourceSourceRef::LocalPath {
                        host_id: "host-local".to_string(),
                        path: PathBuf::from("/tmp/data.txt"),
                    },
                    mount_path: PathBuf::from("/work/data.txt"),
                    read_only: true,
                },
                "runtime:test",
            )
            .await
            .unwrap();

        assert!(mounted.is_none());
    }
}
