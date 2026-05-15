//! HTTP client plumbing for `http_forward`.
//!
//! Owns `mechanics_http_client::Client` construction and one-attempt
//! execution. Retry policy is layered above this in `retry.rs`.

use crate::{
    config::PreparedConfig,
    error::{Error, Result},
    request::{HttpForwardRequest, encode_request_body},
    response::{
        HttpForwardResponse, decode_response_body, extract_exposed_headers, read_response_body,
    },
};
use mechanics_config::HttpMethod;
use mechanics_http_client::{Client, Error as HttpError, HeaderMap, Method};
use std::time::Duration;

pub(crate) fn build_client() -> Result<Client> {
    Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|e| Error::Internal(format!("failed to build HTTP client: {e}")))
}

pub(crate) async fn execute_one_attempt(
    client: &Client,
    prepared: &PreparedConfig,
    request: &HttpForwardRequest,
) -> Result<HttpForwardResponse> {
    let method = prepared.endpoint.method();

    let url = prepared
        .endpoint
        .build_url(&request.url_params, &request.queries)
        .map_err(|e| Error::InvalidRequest(e.to_string()))?;

    let layered_headers = prepared
        .endpoint
        .build_headers(&request.headers)
        .map_err(|e| Error::InvalidRequest(e.to_string()))?;

    let (body_bytes, default_content_type) = encode_request_body(
        method,
        prepared.endpoint.effective_request_body_type(),
        request.body.as_ref(),
    )?;

    let mut builder = client.request(as_http_method(method), url);

    if let Some(timeout_ms) = prepared.endpoint.timeout_ms() {
        builder = builder.timeout(Duration::from_millis(timeout_ms));
    }

    for (name, value) in &layered_headers {
        builder = builder.header(name.as_str(), value.as_str());
    }

    if let Some(content_type) = default_content_type
        && !has_content_type_header(&layered_headers)
    {
        builder = builder.header("content-type", content_type);
    }

    if let Some(bytes) = body_bytes {
        builder = builder.body(bytes);
    }

    let response = builder.send().await.map_err(map_http_error)?;

    let status = response.status().as_u16();
    let ok = (200..=299).contains(&status);
    let response_headers = response.headers().clone();

    let body_bytes = read_response_body(response, prepared.response_max_bytes).await?;
    let body = decode_response_body(prepared.endpoint.response_body_type(), &body_bytes)?;

    let headers = extract_exposed_headers(
        &response_headers,
        prepared.prepared.exposed_response_allowlist(),
    );

    if !ok && !prepared.endpoint.allow_non_2xx_status() {
        return Err(Error::UpstreamNonSuccess {
            status,
            headers,
            body,
            retry_after: retry_after_header(&response_headers),
        });
    }

    Ok(HttpForwardResponse {
        status,
        ok,
        headers,
        body,
    })
}

fn as_http_method(method: HttpMethod) -> Method {
    match method {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
        HttpMethod::Put => Method::PUT,
        HttpMethod::Patch => Method::PATCH,
        HttpMethod::Delete => Method::DELETE,
        HttpMethod::Head => Method::HEAD,
        HttpMethod::Options => Method::OPTIONS,
    }
}

pub(crate) fn map_http_error(error: HttpError) -> Error {
    if error.is_timeout() {
        Error::UpstreamTimeout
    } else {
        Error::UpstreamUnreachable(error.to_string())
    }
}

fn has_content_type_header(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
}

fn retry_after_header(headers: &HeaderMap) -> Option<String> {
    headers.get("retry-after").map(|value| {
        value
            .to_str()
            .map(str::to_owned)
            .unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).into_owned())
    })
}
