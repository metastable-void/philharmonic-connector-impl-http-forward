//! Generic HTTP forwarding implementation for Philharmonic connectors.
//!
//! `http_forward` implements the shared
//! `philharmonic-connector-impl-api::Implementation` trait and reuses
//! `mechanics-config`'s `HttpEndpoint` schema for endpoint definition,
//! request validation, retry policy, and response-shaping rules. This
//! keeps configuration semantics aligned with the existing mechanics
//! endpoint model instead of introducing an implementation-specific DSL.
//!
//! The implementation receives decrypted `config` and `request` JSON
//! objects from the connector service framework, validates them,
//! executes one or more HTTP attempts under endpoint retry policy, and
//! returns a normalized JSON response payload. Failures are mapped to
//! typed `ImplementationError` variants so callers can distinguish
//! invalid input from transport failures and upstream non-success
//! responses.

mod client;
mod config;
mod error;
mod request;
mod response;
mod retry;

pub use crate::config::{HttpForwardConfig, PreparedConfig};
pub use crate::request::HttpForwardRequest;
pub use crate::response::HttpForwardResponse;
pub use philharmonic_connector_impl_api::{
    ConnectorCallContext, Implementation, ImplementationError, JsonValue, async_trait,
};

const NAME: &str = "http_forward";

/// `http_forward` connector implementation.
#[derive(Clone, Debug)]
pub struct HttpForward {
    client: reqwest::Client,
}

impl HttpForward {
    /// Builds an instance with a workspace-standard reqwest client.
    pub fn new() -> Result<Self, ImplementationError> {
        let client = client::build_client().map_err(ImplementationError::from)?;
        Ok(Self { client })
    }

    /// Builds an instance with an externally-configured reqwest client.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Implementation for HttpForward {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(
        &self,
        config: &JsonValue,
        request: &JsonValue,
        ctx: &ConnectorCallContext,
    ) -> Result<JsonValue, ImplementationError> {
        let config: HttpForwardConfig = serde_json::from_value(config.clone())
            .map_err(|e| error::Error::InvalidConfig(e.to_string()))
            .map_err(ImplementationError::from)?;

        let prepared = config
            .prepare()
            .map_err(|e| error::Error::InvalidConfig(e.to_string()))
            .map_err(ImplementationError::from)?;

        let request: HttpForwardRequest = serde_json::from_value(request.clone())
            .map_err(|e| error::Error::InvalidRequest(e.to_string()))
            .map_err(ImplementationError::from)?;

        let response = retry::execute_with_retry(&self.client, &prepared, &request, ctx)
            .await
            .map_err(ImplementationError::from)?;

        serde_json::to_value(response)
            .map_err(|e| error::Error::Internal(e.to_string()))
            .map_err(ImplementationError::from)
    }
}
