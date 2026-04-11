use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedRuntimeSpec {
    pub runtime_key: String,
    pub node_id: String,
    pub create_spec: Value,
}

impl PersistedRuntimeSpec {
    pub fn new(
        runtime_key: impl Into<String>,
        node_id: impl Into<String>,
        create_spec: Value,
    ) -> Self {
        let runtime_key = runtime_key.into();
        let node_id = node_id.into();
        let mut spec = Self {
            runtime_key,
            node_id,
            create_spec,
        };
        spec.ensure_runtime_identity();
        spec
    }

    fn ensure_runtime_identity(&mut self) {
        if let Some(object) = self.create_spec.as_object_mut() {
            object.insert(
                "runtimeKey".to_string(),
                Value::String(self.runtime_key.clone()),
            );
            object.insert("nodeId".to_string(), Value::String(self.node_id.clone()));
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRuntimeSpecWire {
    runtime_key: String,
    node_id: String,
    #[serde(flatten)]
    create_spec: Map<String, Value>,
}

impl Serialize for PersistedRuntimeSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut create_spec = self
            .create_spec
            .as_object()
            .cloned()
            .ok_or_else(|| serde::ser::Error::custom("persisted runtime spec must be an object"))?;
        create_spec.insert(
            "runtimeKey".to_string(),
            Value::String(self.runtime_key.clone()),
        );
        create_spec.insert("nodeId".to_string(), Value::String(self.node_id.clone()));
        Value::Object(create_spec).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PersistedRuntimeSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PersistedRuntimeSpecWire::deserialize(deserializer)?;
        Ok(Self::new(
            wire.runtime_key,
            wire.node_id,
            Value::Object(wire.create_spec),
        ))
    }
}
