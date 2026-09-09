use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::Value;

use super::hash::HashObject;
use super::license::LicenseChoice;

/// A CycloneDX component (`component`), used both for `metadata.component`
/// and for `components[]` or `metadata.tools.components[]`.
///
/// Only the fields most commonly manipulated by this crate's commands are
/// explicitly typed. Everything else (`externalReferences`, `properties`,
/// `evidence`, `cryptoProperties`, future fields...) is preserved losslessly
/// via `extra`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>,
    #[serde(rename = "bom-ref", skip_serializing_if = "Option::is_none")]
    pub bom_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashes: Vec<HashObject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub licenses: Vec<LicenseChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Component {
    pub fn new(component_type: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            component_type: Some(component_type.into()),
            name: Some(name.into()),
            ..Default::default()
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn with_publisher(mut self, publisher: impl Into<String>) -> Self {
        self.publisher = Some(publisher.into());
        self
    }

    pub fn with_purl(mut self, purl: impl Into<String>) -> Self {
        self.purl = Some(purl.into());
        self
    }

    pub fn with_license(mut self, license: LicenseChoice) -> Self {
        self.licenses.push(license);
        self
    }
}
