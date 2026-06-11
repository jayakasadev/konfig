//! Owned snapshot of a single `Config.konfig.io/v1` CRD spec.
//!
//! Lives behind `ArcSwap<ConfigSnapshot>` inside `KonfigConsumer` so reads are
//! lock-free.  `content` is `serde_json::Value` so callers can do
//! `snap.content["risk"]["max"].as_u64()` without re-parsing on every access.

use std::time::Instant;

use kube::core::DynamicObject;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("missing spec field on Config object")]
    MissingSpec,
    #[error("invalid spec JSON: {0}")]
    InvalidSpec(#[from] serde_json::Error),
}

fn default_content() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigSpec {
    pub schema_version: u32,
    #[serde(default = "default_content")]
    pub content: Value,
}

#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub content: Value,
    pub schema_version: u32,
    pub resource_version: String,
    pub loaded_at: Instant,
    /// Set when the watcher loses its K8s connection; cleared on reconnect.
    pub stale_since: Option<Instant>,
}

impl Default for ConfigSnapshot {
    fn default() -> Self {
        Self {
            content: Value::Null,
            schema_version: 0,
            resource_version: String::new(),
            loaded_at: Instant::now(),
            stale_since: None,
        }
    }
}

impl ConfigSnapshot {
    /// Build a snapshot for tests / fallbacks from a literal JSON value.
    /// `schema_version` defaults to 0; `resource_version` is empty.
    pub fn compiled_in(content: Value) -> Self {
        Self {
            content,
            schema_version: 0,
            resource_version: String::new(),
            loaded_at: Instant::now(),
            stale_since: None,
        }
    }

    pub fn from_spec(spec: ConfigSpec, resource_version: String) -> Self {
        Self {
            content: spec.content,
            schema_version: spec.schema_version,
            resource_version,
            loaded_at: Instant::now(),
            stale_since: None,
        }
    }

    pub fn with_stale_since(mut self, when: Instant) -> Self {
        self.stale_since = Some(when);
        self
    }
}

/// Parse a `DynamicObject` (Config CRD) into a `ConfigSnapshot`.
///
/// Returns `None` if the spec is missing or fails to deserialise — caller
/// retains the previous snapshot in that case (CP semantics).
pub fn parse_config_object(obj: &DynamicObject) -> Option<ConfigSnapshot> {
    let resource_version = obj.metadata.resource_version.clone().unwrap_or_default();
    let name = obj.metadata.name.as_deref().unwrap_or("<unknown>");

    let spec_value = obj.data.get("spec")?;
    let spec: ConfigSpec = match serde_json::from_value(spec_value.clone()) {
        Ok(spec) => spec,
        Err(e) => {
            warn!(name = %name, "konfig-consumer: failed to parse Config spec: {e}");
            return None;
        }
    };

    Some(ConfigSnapshot::from_spec(spec, resource_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::ApiResource;
    use serde_json::json;

    fn ar() -> ApiResource {
        ApiResource {
            group: "konfig.io".to_string(),
            version: "v1".to_string(),
            api_version: "konfig.io/v1".to_string(),
            kind: "Config".to_string(),
            plural: "configs".to_string(),
        }
    }

    fn make_obj(name: &str, schema_version: u32, content: Value) -> DynamicObject {
        let mut obj = DynamicObject::new(name, &ar());
        obj.metadata.name = Some(name.to_string());
        obj.metadata.namespace = Some("default".to_string());
        obj.metadata.resource_version = Some("rv-7".to_string());
        obj.data = json!({
            "spec": {
                "schema_version": schema_version,
                "content": content,
            }
        });
        obj
    }

    #[test]
    fn parse_valid_object() {
        let obj = make_obj("risk-config", 5, json!({"risk": {"max": 100}}));
        let snap = parse_config_object(&obj).expect("parses");
        assert_eq!(snap.schema_version, 5);
        assert_eq!(snap.content["risk"]["max"], 100);
        assert_eq!(snap.resource_version, "rv-7");
        assert!(snap.stale_since.is_none());
    }

    #[test]
    fn parse_missing_spec_returns_none() {
        let mut obj = DynamicObject::new("x", &ar());
        obj.data = json!({});
        assert!(parse_config_object(&obj).is_none());
    }

    #[test]
    fn parse_default_content_when_field_missing() {
        let mut obj = DynamicObject::new("x", &ar());
        obj.data = json!({"spec": {"schema_version": 2}});
        let snap = parse_config_object(&obj).expect("parses");
        assert_eq!(snap.schema_version, 2);
        assert!(snap.content.is_object());
    }

    #[test]
    fn parse_invalid_spec_returns_none() {
        let mut obj = DynamicObject::new("x", &ar());
        obj.data = json!({"spec": {"schema_version": "not-a-number"}});
        assert!(parse_config_object(&obj).is_none());
    }

    #[test]
    fn compiled_in_constructs_from_value() {
        let snap = ConfigSnapshot::compiled_in(json!({"k": 1}));
        assert_eq!(snap.content["k"], 1);
        assert_eq!(snap.schema_version, 0);
        assert!(snap.stale_since.is_none());
    }
}
