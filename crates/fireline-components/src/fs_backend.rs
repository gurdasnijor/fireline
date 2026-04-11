//! ACP filesystem backend component.
//!
//! This is intentionally narrower than Durable Streams' experimental
//! `stream-fs` package. `RuntimeStreamFileBackend` is a single-runtime,
//! single-writer artifact log backed by one Fireline state stream. It is not a
//! collaborative filesystem: no rename, no directory metadata, no multi-writer
//! conflict resolution, and no attempt to present stronger semantics than
//! append-only projection can support.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset, Producer};
use fireline_conductor::runtime::MountedResource;
use sacp::schema::{
    ReadTextFileRequest, ReadTextFileResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use sacp::{Agent, Conductor, ConnectTo, Proxy};
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait FileBackend: Send + Sync {
    async fn read(&self, path: &Path) -> Result<Vec<u8>>;
    async fn write(&self, path: &Path, content: &[u8]) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum FsBackendConfig {
    Local,
    RuntimeStream,
}

impl FsBackendConfig {
    pub fn local() -> Self {
        Self::Local
    }

    pub fn runtime_stream() -> Self {
        Self::RuntimeStream
    }
}

#[derive(Clone)]
pub struct FsBackendComponent {
    backend: Arc<dyn FileBackend>,
    state_producer: Producer,
}

impl FsBackendComponent {
    pub fn new(backend: Arc<dyn FileBackend>, state_producer: Producer) -> Self {
        Self {
            backend,
            state_producer,
        }
    }

    pub async fn handle_read_text_file(
        &self,
        request: &ReadTextFileRequest,
    ) -> Result<ReadTextFileResponse> {
        let bytes = self
            .backend
            .read(&request.path)
            .await
            .with_context(|| format!("fs backend read {}", request.path.display()))?;
        let content = String::from_utf8(bytes).with_context(|| {
            format!(
                "fs backend read {} returned non-utf8 data",
                request.path.display()
            )
        })?;
        Ok(ReadTextFileResponse::new(apply_read_window(
            &content,
            request.line,
            request.limit,
        )))
    }

    pub async fn handle_write_text_file(
        &self,
        request: &WriteTextFileRequest,
    ) -> Result<WriteTextFileResponse> {
        self.backend
            .write(&request.path, request.content.as_bytes())
            .await
            .with_context(|| format!("fs backend write {}", request.path.display()))?;
        append_fs_op_event(
            &self.state_producer,
            &request.session_id.to_string(),
            &request.path,
            &request.content,
        );
        self.state_producer
            .flush()
            .await
            .context("flush fs backend fs_op event")?;
        Ok(WriteTextFileResponse::new())
    }
}

impl ConnectTo<Conductor> for FsBackendComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let read_component = self.clone();
        let write_component = self.clone();

        sacp::Proxy
            .builder()
            .name("fireline-fs-backend")
            .on_receive_request_from(
                Agent,
                {
                    let read_component = read_component.clone();
                    async move |request: ReadTextFileRequest, responder, _cx| {
                        let response = read_component
                            .handle_read_text_file(&request)
                            .await
                            .map_err(|error| {
                                sacp::util::internal_error(format!(
                                    "fs backend read {}: {error:#}",
                                    request.path.display()
                                ))
                            })?;
                        responder.respond(response)
                    }
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request_from(
                Agent,
                {
                    let write_component = write_component.clone();
                    async move |request: WriteTextFileRequest, responder, _cx| {
                        let response = write_component
                            .handle_write_text_file(&request)
                            .await
                            .map_err(|error| {
                                sacp::util::internal_error(format!(
                                    "fs backend write {}: {error:#}",
                                    request.path.display()
                                ))
                            })?;
                        responder.respond(response)
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

#[derive(Debug, Clone, Default)]
pub struct LocalFileBackend {
    mounted_resources: Vec<MountedResource>,
}

impl LocalFileBackend {
    pub fn new(mounted_resources: Vec<MountedResource>) -> Self {
        Self { mounted_resources }
    }

    fn resolve_path(&self, requested: &Path) -> Option<(PathBuf, bool)> {
        if requested.exists() {
            return Some((requested.to_path_buf(), false));
        }

        self.mounted_resources.iter().find_map(|mount| {
            let resolved = resolve_mount_path(mount, requested)?;
            Some((resolved, mount.read_only))
        })
    }
}

#[async_trait]
impl FileBackend for LocalFileBackend {
    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let (resolved, _) = self
            .resolve_path(path)
            .ok_or_else(|| anyhow!("file '{}' not found", path.display()))?;
        tokio::fs::read(&resolved)
            .await
            .with_context(|| format!("read local file backend path {}", resolved.display()))
    }

    async fn write(&self, path: &Path, content: &[u8]) -> Result<()> {
        let (resolved, read_only) = self
            .resolve_path(path)
            .unwrap_or_else(|| (path.to_path_buf(), false));
        if read_only {
            return Err(anyhow!(
                "resource path '{}' is mounted read-only",
                path.display()
            ));
        }
        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("create local file backend parent {}", parent.display())
            })?;
        }
        tokio::fs::write(&resolved, content)
            .await
            .with_context(|| format!("write local file backend path {}", resolved.display()))
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeStreamFileBackend {
    state_stream_url: String,
}

/// Backwards-compatible alias kept for the executable spec and docs.
///
/// This is **not** Durable Streams' upstream `stream-fs`. It is the much more
/// constrained single-runtime backend described in the module docs above.
pub type SessionLogFileBackend = RuntimeStreamFileBackend;

impl RuntimeStreamFileBackend {
    pub fn new(state_stream_url: impl Into<String>) -> Self {
        Self {
            state_stream_url: state_stream_url.into(),
        }
    }
}

#[async_trait]
impl FileBackend for RuntimeStreamFileBackend {
    async fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let Some(record) = latest_stream_file_record(&self.state_stream_url, path).await? else {
            return Err(anyhow!(
                "runtime stream file '{}' not found in {}",
                path.display(),
                self.state_stream_url
            ));
        };
        Ok(record.content.into_bytes())
    }

    async fn write(&self, path: &Path, content: &[u8]) -> Result<()> {
        let content = String::from_utf8(content.to_vec())
            .with_context(|| format!("runtime stream file '{}' must be utf-8", path.display()))?;
        let producer = state_stream_producer(&self.state_stream_url, "runtime-stream-file");
        producer.append_json(&StateEnvelope {
            entity_type: "runtime_stream_file",
            key: path_key(path),
            headers: StateHeaders {
                operation: "upsert",
            },
            value: Some(RuntimeStreamFileRecord {
                path: path_key(path),
                content,
                ts_ms: now_ms(),
            }),
        });
        producer
            .flush()
            .await
            .context("flush runtime stream file write")?;
        Ok(())
    }
}

fn resolve_mount_path(mount: &MountedResource, requested: &Path) -> Option<PathBuf> {
    if requested == mount.mount_path {
        return Some(mount.host_path.clone());
    }

    let relative = requested.strip_prefix(&mount.mount_path).ok()?;
    if relative.as_os_str().is_empty() {
        Some(mount.host_path.clone())
    } else {
        Some(mount.host_path.join(relative))
    }
}

fn apply_read_window(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return content.to_string();
    }

    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let end = limit.map(|limit| start + limit as usize);
    content
        .lines()
        .skip(start)
        .take(
            end.map(|end| end.saturating_sub(start))
                .unwrap_or(usize::MAX),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_fs_op_event(producer: &Producer, session_id: &str, path: &Path, content: &str) {
    producer.append_json(&StateEnvelope {
        entity_type: "fs_op",
        key: format!("{session_id}:{}", path_key(path)),
        headers: StateHeaders {
            operation: "upsert",
        },
        value: Some(FsOpRecord {
            session_id: session_id.to_string(),
            path: path_key(path),
            op: "write".to_string(),
            content: content.to_string(),
            ts_ms: now_ms(),
        }),
    });
}

async fn latest_stream_file_record(
    state_stream_url: &str,
    path: &Path,
) -> Result<Option<RuntimeStreamFileRecord>> {
    let mut latest = None;
    for envelope in read_state_envelopes(state_stream_url).await? {
        if envelope.entity_type == "runtime_stream_file"
            && let Some(value) = envelope.value
        {
            let record: RuntimeStreamFileRecord =
                serde_json::from_value(value).context("decode runtime stream file record")?;
            if record.path == path_key(path) {
                latest = Some(record);
            }
            continue;
        }

        if envelope.entity_type == "fs_op"
            && let Some(value) = envelope.value
        {
            let record: FsOpRecord =
                serde_json::from_value(value).context("decode fs op record")?;
            if record.path == path_key(path) && record.op == "write" {
                latest = Some(RuntimeStreamFileRecord {
                    path: record.path,
                    content: record.content,
                    ts_ms: record.ts_ms,
                });
            }
        }
    }
    Ok(latest)
}

async fn read_state_envelopes(state_stream_url: &str) -> Result<Vec<RawStateEnvelope>> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(state_stream_url);
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Off)
        .build()
        .context("build durable stream reader")?;
    let mut envelopes = Vec::new();
    while let Some(chunk) = reader
        .next_chunk()
        .await
        .context("read durable stream chunk")?
    {
        if chunk.data.is_empty() {
            continue;
        }
        let events = serde_json::from_slice::<Vec<serde_json::Value>>(&chunk.data)
            .context("decode durable stream chunk")?;
        for event in events {
            if let Ok(envelope) = serde_json::from_value::<RawStateEnvelope>(event) {
                envelopes.push(envelope);
            }
        }
        if chunk.up_to_date {
            break;
        }
    }
    Ok(envelopes)
}

fn state_stream_producer(state_stream_url: &str, producer_prefix: &str) -> Producer {
    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(format!("{producer_prefix}-{}", uuid::Uuid::new_v4()))
        .content_type("application/json")
        .build()
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

#[derive(Debug, Clone, Deserialize)]
struct RawStateEnvelope {
    #[serde(rename = "type")]
    entity_type: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
struct StateHeaders {
    operation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct StateEnvelope<T> {
    #[serde(rename = "type")]
    entity_type: &'static str,
    key: String,
    headers: StateHeaders,
    value: Option<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStreamFileRecord {
    pub path: String,
    pub content: String,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsOpRecord {
    pub session_id: String,
    pub path: String,
    pub op: String,
    pub content: String,
    pub ts_ms: i64,
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::LocalFileBackend;
    use crate::fs_backend::FileBackend;

    #[tokio::test]
    async fn local_file_backend_reads_through_mount_mapping() {
        let source_dir =
            std::env::temp_dir().join(format!("fireline-fs-backend-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("hello.txt"), "hello").unwrap();

        let backend = LocalFileBackend::new(vec![fireline_conductor::runtime::MountedResource {
            host_path: source_dir.clone(),
            mount_path: PathBuf::from("/work"),
            read_only: true,
        }]);

        let bytes = backend.read(Path::new("/work/hello.txt")).await.unwrap();
        assert_eq!(bytes, b"hello");

        let _ = std::fs::remove_dir_all(source_dir);
    }
}
