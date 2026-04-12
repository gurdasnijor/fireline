use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stream envelope aligned to Durable Streams `STATE-PROTOCOL`.
///
/// Change messages carry `type`, `key`, `value?`, `old_value?`, and
/// `headers.operation`. Control messages carry only `headers.control` with an
/// optional `headers.offset`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct StateEnvelope {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_value: Option<Value>,
    pub headers: StateHeaders,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeOperation {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ControlKind {
    SnapshotStart,
    SnapshotEnd,
    Reset,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StateHeaders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<ChangeOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<String>,
}

impl StateEnvelope {
    pub fn change_operation(&self) -> Option<ChangeOperation> {
        self.headers.operation.clone()
    }

    pub fn control_kind(&self) -> Option<ControlKind> {
        self.headers.control.clone()
    }

    pub fn is_change(&self) -> bool {
        self.headers.operation.is_some()
    }

    pub fn entity_type(&self) -> Option<&str> {
        self.entity_type.as_deref()
    }

    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }
}

pub trait StreamProjection: Send + Sync {
    fn apply(&self, envelope: &StateEnvelope) -> Result<()>;

    fn reset(&self) -> Result<()> {
        Ok(())
    }
}
