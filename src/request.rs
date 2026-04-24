//! Request model and request-body encoding for `http_forward`.
//!
//! The wire form is camelCase to match the script-facing protocol.
//! This module deserializes that shape and normalizes request-body
//! bytes for the outbound HTTP call.

use crate::error::{Error, Result};
use base64::Engine;
use mechanics_config::{EndpointBodyType, HttpMethod};
use philharmonic_connector_impl_api::JsonValue;
use std::collections::HashMap;

/// One call request payload for `http_forward`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpForwardRequest {
    /// URL template slot values.
    #[serde(default)]
    pub url_params: HashMap<String, String>,
    /// Slotted query values keyed by slot name.
    #[serde(default)]
    pub queries: HashMap<String, String>,
    /// Per-call header overrides.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Optional body payload interpreted by endpoint body mode.
    #[serde(default)]
    pub body: Option<JsonValue>,
}

pub(crate) fn encode_request_body(
    method: HttpMethod,
    body_type: EndpointBodyType,
    body: Option<&JsonValue>,
) -> Result<(Option<Vec<u8>>, Option<&'static str>)> {
    if !method.supports_request_body() {
        if body.is_some() {
            return Err(Error::InvalidRequest(format!(
                "HTTP {} endpoint does not accept a request body",
                method.as_str()
            )));
        }
        return Ok((None, None));
    }

    match body_type {
        EndpointBodyType::Json => {
            let payload = match body {
                Some(value) => value,
                None => &JsonValue::Null,
            };
            let encoded = serde_json::to_vec(payload)
                .map_err(|e| Error::InvalidRequest(format!("invalid json request body: {e}")))?;
            Ok((Some(encoded), Some("application/json")))
        }
        EndpointBodyType::Utf8 => match body {
            None => Ok((None, None)),
            Some(value) => {
                let text = value.as_str().ok_or_else(|| {
                    Error::InvalidRequest(
                        "request_body_type `utf8` requires request.body to be a JSON string"
                            .to_owned(),
                    )
                })?;
                Ok((
                    Some(text.as_bytes().to_vec()),
                    Some("text/plain; charset=utf-8"),
                ))
            }
        },
        EndpointBodyType::Bytes => match body {
            None => Ok((None, None)),
            Some(value) => {
                let encoded = value.as_str().ok_or_else(|| {
                    Error::InvalidRequest(
                        "request_body_type `bytes` requires request.body to be a base64 JSON string"
                            .to_owned(),
                    )
                })?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|e| {
                        Error::InvalidRequest(format!(
                            "request_body_type `bytes` expected base64 string: {e}"
                        ))
                    })?;
                Ok((Some(bytes), Some("application/octet-stream")))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_camelcase_wire_form() {
        let value = json!({
            "urlParams": {"userId": "u_123"},
            "queries": {"trace": "abc"},
            "headers": {"Idempotency-Key": "k_1"},
            "body": {"event": "created"}
        });

        let req = serde_json::from_value::<HttpForwardRequest>(value).unwrap();
        assert_eq!(req.url_params.get("userId").unwrap(), "u_123");
        assert_eq!(req.queries.get("trace").unwrap(), "abc");
        assert_eq!(req.headers.get("Idempotency-Key").unwrap(), "k_1");
        assert_eq!(req.body.unwrap()["event"], json!("created"));
    }

    #[test]
    fn deserialize_rejects_unknown_fields() {
        let value = json!({
            "urlParams": {},
            "queries": {},
            "headers": {},
            "body": null,
            "unknown": true
        });

        let err = serde_json::from_value::<HttpForwardRequest>(value).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }
}
