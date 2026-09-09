use serde::{Deserialize, Serialize};

/// A CycloneDX `hash` object (`{ "alg": "...", "content": "..." }`).
///
/// `alg` is deliberately a `String` rather than a closed enum: the
/// CycloneDX specification regularly adds new hashing algorithms, and we
/// don't want to have to update this crate on every minor schema change
/// just to stay able to read/write an SBOM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashObject {
    pub alg: String,
    pub content: String,
}

impl HashObject {
    pub fn sha256(content: impl Into<String>) -> Self {
        Self {
            alg: "SHA-256".to_string(),
            content: content.into(),
        }
    }
}
