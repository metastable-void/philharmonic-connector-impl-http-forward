//! Configuration model for `http_forward`.
//!
//! `HttpForwardConfig` is intentionally thin: it wraps the shared
//! `mechanics-config` `HttpEndpoint` schema rather than introducing a
//! second endpoint DSL. Runtime preparation validates endpoint shape
//! and precomputes reusable data.

use mechanics_config::{HttpEndpoint, PreparedHttpEndpoint};
use std::io::{Error, ErrorKind};

/// Top-level config payload for the `http_forward` implementation.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpForwardConfig {
    /// Endpoint definition reused directly from `mechanics-config`.
    pub endpoint: HttpEndpoint,
}

impl HttpForwardConfig {
    /// Validates endpoint config and prepares runtime caches.
    pub fn prepare(&self) -> std::io::Result<PreparedConfig> {
        if self.endpoint.response_max_bytes().is_none() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "missing response_max_bytes",
            ));
        }

        self.endpoint.validate_config()?;
        let prepared = self.endpoint.prepare_runtime()?;

        let response_max_bytes = self
            .endpoint
            .response_max_bytes()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "missing response_max_bytes"))?;

        Ok(PreparedConfig {
            endpoint: self.endpoint.clone(),
            prepared,
            response_max_bytes,
        })
    }
}

/// Runtime-ready endpoint config with prepared lookup structures.
#[derive(Debug, Clone)]
pub struct PreparedConfig {
    /// Original endpoint payload for method/timeouts/policies.
    pub endpoint: HttpEndpoint,
    /// Prepared endpoint internals from `mechanics-config`.
    pub prepared: PreparedHttpEndpoint,
    /// Mandatory response size cap used for streamed body checks.
    pub response_max_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_config_json() -> serde_json::Value {
        json!({
            "endpoint": {
                "method": "post",
                "url_template": "https://example.test/messages/{id}",
                "url_param_specs": {
                    "id": {
                        "default": null,
                        "min_bytes": 1,
                        "max_bytes": 64
                    }
                },
                "query_specs": [],
                "headers": {
                    "Authorization": "Bearer secret"
                },
                "overridable_request_headers": [],
                "exposed_response_headers": ["X-Request-Id"],
                "request_body_type": "json",
                "response_body_type": "json",
                "response_max_bytes": 1024,
                "timeout_ms": 1000,
                "allow_non_2xx_status": false,
                "retry_policy": {
                    "max_attempts": 1,
                    "base_backoff_ms": 100,
                    "max_backoff_ms": 100,
                    "max_retry_delay_ms": 1000,
                    "rate_limit_backoff_ms": 100,
                    "retry_on_io_errors": true,
                    "retry_on_timeout": true,
                    "respect_retry_after": true,
                    "retry_on_status": [500]
                }
            }
        })
    }

    #[test]
    fn deserialize_rejects_unknown_fields() {
        let value = json!({
            "endpoint": {
                "method": "get",
                "url_template": "https://example.test",
                "headers": {},
                "response_max_bytes": 1024,
                "timeout_ms": 1000,
                "extra": true
            }
        });

        let err = serde_json::from_value::<HttpForwardConfig>(value).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn prepare_runtime_invalid_url_template_rejected() {
        let mut value = valid_config_json();
        value["endpoint"]["url_template"] = json!("https://example.test/messages/{id");

        let cfg = serde_json::from_value::<HttpForwardConfig>(value).unwrap();
        let err = cfg.prepare().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn missing_response_max_bytes_is_rejected() {
        let mut value = valid_config_json();
        if let serde_json::Value::Object(endpoint) = &mut value["endpoint"] {
            endpoint.remove("response_max_bytes");
        }

        let cfg = serde_json::from_value::<HttpForwardConfig>(value).unwrap();
        let err = cfg.prepare().unwrap_err();
        assert_eq!(err.to_string(), "missing response_max_bytes");
    }
}
