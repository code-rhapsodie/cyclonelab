//! Minimal JSON-over-HTTP client abstraction, so forge implementations can be
//! tested with recorded responses instead of the network.

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Fetches JSON documents. Implemented by [`UreqClient`] in production and by
/// fakes in tests.
pub trait HttpClient {
    /// GETs `url` with `headers` and parses the body as JSON. `Ok(None)` means
    /// HTTP 404; every other failure (network, other status, rate limit,
    /// invalid JSON) is an error.
    fn get_json(&self, url: &str, headers: &[(String, String)]) -> Result<Option<Value>>;
}

/// [`HttpClient`] backed by `ureq`.
pub struct UreqClient;

impl HttpClient for UreqClient {
    fn get_json(&self, url: &str, headers: &[(String, String)]) -> Result<Option<Value>> {
        let mut request = ureq::get(url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        match request.call() {
            Ok(response) => {
                let body = response
                    .into_string()
                    .with_context(|| format!("Failed to read the response of '{url}'"))?;
                Ok(Some(serde_json::from_str(&body).with_context(|| {
                    format!("Invalid JSON received from '{url}'")
                })?))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(ureq::Error::Status(code @ (403 | 429), response))
                if response.header("x-ratelimit-remaining") == Some("0") || code == 429 =>
            {
                bail!(
                    "Rate limit exceeded requesting '{url}' (HTTP {code}); set the forge's API token environment variable to raise it"
                )
            }
            Err(ureq::Error::Status(code, _)) => bail!("HTTP {code} requesting '{url}'"),
            Err(error) => Err(error).with_context(|| format!("Failed to request '{url}'")),
        }
    }
}
