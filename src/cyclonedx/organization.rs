use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::Value;

/// A CycloneDX `organizationalEntity`, used for `metadata.manufacturer`
/// (schema >= 1.7) among other places.
///
/// Fields not explicitly modeled (`address`, `contact`, future fields...)
/// are kept as-is in `extra` thanks to `#[serde(flatten)]`, which allows
/// deserializing then reserializing an SBOM without losing information even
/// as the schema evolves.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationalEntity {
    #[serde(rename = "bom-ref", skip_serializing_if = "Option::is_none")]
    pub bom_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub url: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_unmodeled_fields() {
        let json = serde_json::json!({
            "name": "Example Inc.",
            "url": ["https://example.com"],
            "address": {"country": "France"},
            "contact": [{"name": "Support"}]
        });

        let manufacturer: OrganizationalEntity = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(manufacturer.name.as_deref(), Some("Example Inc."));
        assert_eq!(manufacturer.url, vec!["https://example.com".to_string()]);
        assert!(manufacturer.extra.contains_key("address"));
        assert!(manufacturer.extra.contains_key("contact"));

        let reserialized = serde_json::to_value(&manufacturer).unwrap();
        assert_eq!(reserialized, json);
    }
}
