use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub type HostId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamFsMode {
    SnapshotReadOnly,
    LiveReadOnly,
    LiveReadWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResourceSourceRef {
    LocalPath {
        host_id: HostId,
        path: PathBuf,
    },
    S3 {
        bucket: String,
        key: String,
        region: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint_url: Option<String>,
    },
    Gcs {
        bucket: String,
        key: String,
    },
    DockerVolume {
        host_id: HostId,
        volume_name: String,
        path_within_volume: PathBuf,
    },
    DurableStreamBlob {
        stream: String,
        key: String,
    },
    StreamFs {
        source_ref: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<String>,
        mode: StreamFsMode,
    },
    OciImageLayer {
        image: String,
        path: PathBuf,
    },
    GitRepo {
        url: String,
        r#ref: String,
        path: PathBuf,
    },
    HttpUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishedResourceRef {
    pub source_ref: ResourceSourceRef,
    pub mount_path: PathBuf,
    #[serde(default = "default_read_only")]
    pub read_only: bool,
}

pub type ResourceRef = PublishedResourceRef;

const fn default_read_only() -> bool {
    true
}
