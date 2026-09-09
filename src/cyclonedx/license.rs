use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::Value;

/// License details (`licenses[].license` in the CycloneDX schema).
///
/// Fields not explicitly modeled (`licensing`, `properties`, future 1.8+
/// fields, ...) are kept as-is in `extra` thanks to `#[serde(flatten)]`,
/// which allows deserializing then reserializing an SBOM without losing
/// information even as the schema evolves.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledgement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `licenses[]` accepts either `{ "license": {...} }` or `{ "expression": "..." }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LicenseChoice {
    Single {
        license: LicenseInfo,
    },
    Expression {
        expression: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        acknowledgement: Option<String>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
}

impl LicenseChoice {
    pub fn named(
        name: impl Into<String>,
        url: impl Into<String>,
        acknowledgement: impl Into<String>,
    ) -> Self {
        LicenseChoice::Single {
            license: LicenseInfo {
                name: Some(name.into()),
                url: Some(url.into()),
                acknowledgement: Some(acknowledgement.into()),
                ..Default::default()
            },
        }
    }
}
