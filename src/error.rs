//! Internal error model for `http_forward`.
//!
//! This module keeps implementation-local failure states typed and
//! explicit, then maps them to the connector-wide
//! `ImplementationError` boundary type consumed by the framework.

use philharmonic_connector_impl_api::{ImplementationError, JsonValue};
use std::collections::BTreeMap;

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub(crate) enum Error {
    #[error("{0}")]
    InvalidConfig(String),

    #[error("{0}")]
    InvalidRequest(String),

    #[error("upstream returned non-success status {status}")]
    UpstreamNonSuccess {
        status: u16,
        headers: BTreeMap<String, String>,
        body: JsonValue,
        retry_after: Option<String>,
    },

    #[error("{0}")]
    UpstreamUnreachable(String),

    #[error("upstream timeout")]
    UpstreamTimeout,

    #[error("response too large: limit={limit} actual={actual}")]
    ResponseTooLarge { limit: usize, actual: usize },

    #[error("{0}")]
    Internal(String),
}

impl From<Error> for ImplementationError {
    fn from(value: Error) -> Self {
        match value {
            Error::InvalidConfig(detail) => ImplementationError::InvalidConfig { detail },
            Error::InvalidRequest(detail) => ImplementationError::InvalidRequest { detail },
            Error::UpstreamNonSuccess {
                status,
                headers,
                body,
                ..
            } => {
                let payload = serde_json::json!({
                    "status": status,
                    "headers": headers,
                    "body": body,
                });
                match serde_json::to_string(&payload) {
                    Ok(encoded) => ImplementationError::UpstreamError {
                        status,
                        body: encoded,
                    },
                    Err(err) => ImplementationError::Internal {
                        detail: format!(
                            "failed to encode upstream-error payload for status {status}: {err}"
                        ),
                    },
                }
            }
            Error::UpstreamUnreachable(detail) => {
                ImplementationError::UpstreamUnreachable { detail }
            }
            Error::UpstreamTimeout => ImplementationError::UpstreamTimeout,
            Error::ResponseTooLarge { limit, actual } => {
                ImplementationError::ResponseTooLarge { limit, actual }
            }
            Error::Internal(detail) => ImplementationError::Internal { detail },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_internal_variant_maps_to_wire() {
        let invalid_config = ImplementationError::from(Error::InvalidConfig("bad cfg".to_owned()));
        assert_eq!(
            invalid_config,
            ImplementationError::InvalidConfig {
                detail: "bad cfg".to_owned(),
            }
        );

        let invalid_request =
            ImplementationError::from(Error::InvalidRequest("bad req".to_owned()));
        assert_eq!(
            invalid_request,
            ImplementationError::InvalidRequest {
                detail: "bad req".to_owned(),
            }
        );

        let upstream = ImplementationError::from(Error::UpstreamNonSuccess {
            status: 404,
            headers: BTreeMap::from([("x-request-id".to_owned(), "abc".to_owned())]),
            body: serde_json::json!({"error": "missing"}),
            retry_after: None,
        });
        let ImplementationError::UpstreamError { status, body } = upstream else {
            panic!("expected upstream error mapping");
        };
        assert_eq!(status, 404);
        let payload: JsonValue = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["status"], serde_json::json!(404));
        assert_eq!(payload["headers"]["x-request-id"], serde_json::json!("abc"));
        assert_eq!(payload["body"]["error"], serde_json::json!("missing"));

        let unreachable = ImplementationError::from(Error::UpstreamUnreachable("dns".to_owned()));
        assert_eq!(
            unreachable,
            ImplementationError::UpstreamUnreachable {
                detail: "dns".to_owned(),
            }
        );

        let timeout = ImplementationError::from(Error::UpstreamTimeout);
        assert_eq!(timeout, ImplementationError::UpstreamTimeout);

        let too_large = ImplementationError::from(Error::ResponseTooLarge {
            limit: 8,
            actual: 13,
        });
        assert_eq!(
            too_large,
            ImplementationError::ResponseTooLarge {
                limit: 8,
                actual: 13,
            }
        );

        let internal = ImplementationError::from(Error::Internal("boom".to_owned()));
        assert_eq!(
            internal,
            ImplementationError::Internal {
                detail: "boom".to_owned(),
            }
        );
    }
}
