use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::Value;

use super::component::Component;
use super::organization::OrganizationalEntity;

/// `metadata.tools`, in its object form (CycloneDX schema >= 1.5).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tools {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub services: Option<Vec<Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Tools {
    /// Fully replaces the tool-components list with a single component.
    /// This is the behavior of the original PowerShell script
    /// (`Add-Member ... -Force`), kept identical here.
    pub fn set_single_component(component: Component) -> Self {
        Tools {
            components: Some(vec![component]),
            services: None,
            extra: Map::new(),
        }
    }
}

/// The BOM's `metadata`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Tools>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<Component>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<OrganizationalEntity>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Root of a CycloneDX document.
///
/// `components`, `dependencies`, `services`, etc. are deliberately not all
/// typed: this crate currently focuses on `metadata` manipulation. They are
/// preserved losslessly via `extra` and can be typed as new commands need
/// them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bom {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub bom_format: String,
    pub spec_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Bom {
    pub fn metadata_mut(&mut self) -> &mut Metadata {
        self.metadata.get_or_insert_with(Metadata::default)
    }
}
