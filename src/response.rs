//! Response shaping and body decoding for `http_forward`.
//!
//! Upstream HTTP responses are converted into the stable connector
//! wire shape `{status, ok, headers, body}`. Header exposure follows
//! the endpoint allowlist, and body decoding follows `EndpointBodyType`.

use crate::client::map_http_error;
use crate::error::{Error, Result};
use base64::Engine;
use mechanics_config::EndpointBodyType;
use mechanics_http_client::{HeaderMap, Response};
use philharmonic_connector_impl_api::JsonValue;
use std::collections::{BTreeMap, HashSet};

/// Success response envelope returned by `http_forward`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpForwardResponse {
    /// Upstream HTTP status code.
    pub status: u16,
    /// Convenience status classification (`true` for 2xx).
    pub ok: bool,
    /// Exposed response headers, normalized to lowercase names.
    pub headers: BTreeMap<String, String>,
    /// Decoded response body according to endpoint response body type.
    pub body: JsonValue,
}

pub(crate) fn extract_exposed_headers(
    headers: &HeaderMap,
    allowlist: &HashSet<String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();

    for name in allowlist {
        let values = headers
            .get_all(name)
            .iter()
            .map(|entry| {
                entry
                    .to_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|_| String::from_utf8_lossy(entry.as_bytes()).into_owned())
            })
            .collect::<Vec<_>>();
        if !values.is_empty() {
            out.insert(name.to_ascii_lowercase(), values.join(", "));
        }
    }

    out
}

pub(crate) async fn read_response_body(response: Response, limit: usize) -> Result<Vec<u8>> {
    if let Some(content_length) = response.content_length()
        && content_length > u64::try_from(limit).unwrap_or(u64::MAX)
    {
        let actual = match usize::try_from(content_length) {
            Ok(value) => value,
            Err(_) => usize::MAX,
        };
        return Err(Error::ResponseTooLarge { limit, actual });
    }

    match response.bytes_with_cap(limit).await {
        Ok(bytes) => Ok(bytes.to_vec()),
        Err(mechanics_http_client::Error::BodyTooLarge { limit, seen }) => {
            Err(Error::ResponseTooLarge {
                limit,
                actual: seen.saturating_add(1),
            })
        }
        Err(other) => Err(map_http_error(other)),
    }
}

pub(crate) fn decode_response_body(body_type: EndpointBodyType, bytes: &[u8]) -> Result<JsonValue> {
    match body_type {
        EndpointBodyType::Json => {
            if bytes.is_empty() {
                Ok(JsonValue::Null)
            } else {
                serde_json::from_slice(bytes).map_err(|e| {
                    Error::Internal(format!(
                        "failed to decode json response body as configured by response_body_type=json: {e}"
                    ))
                })
            }
        }
        EndpointBodyType::Utf8 => {
            let text = std::str::from_utf8(bytes).map_err(|e| {
                Error::Internal(format!(
                    "failed to decode utf8 response body as configured by response_body_type=utf8: {e}"
                ))
            })?;
            Ok(JsonValue::String(text.to_owned()))
        }
        EndpointBodyType::Bytes => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            Ok(JsonValue::String(encoded))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mechanics_http_client::{HeaderName, HeaderValue};

    #[test]
    fn header_keys_lowercased() {
        let mut headers = HeaderMap::new();
        headers.append(
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_static("abc"),
        );
        headers.append(
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_static("def"),
        );

        let allowlist = HashSet::from(["X-Request-Id".to_ascii_lowercase()]);
        let exposed = extract_exposed_headers(&headers, &allowlist);

        assert_eq!(exposed.get("x-request-id").unwrap(), "abc, def");
    }
}
