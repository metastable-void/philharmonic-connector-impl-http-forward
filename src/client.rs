//! HTTP client plumbing for `http_forward`.
//!
//! This module owns reqwest client construction and one-attempt
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
use std::time::Duration;

pub(crate) fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .build()
        .map_err(|e| Error::Internal(format!("failed to build reqwest client: {e}")))
}

pub(crate) async fn execute_one_attempt(
    client: &reqwest::Client,
    prepared: &PreparedConfig,
    request: &HttpForwardRequest,
) -> Result<HttpForwardResponse> {
    let method = prepared.endpoint.method();

    let url = prepared
        .endpoint
        .build_url(&request.url_params, &request.queries)
        .map_err(|e| Error::InvalidRequest(e.to_string()))?;
    let url = reqwest::Url::parse(&url)
        .map_err(|e| Error::InvalidRequest(format!("invalid outbound URL: {e}")))?;

    let layered_headers = prepared
        .endpoint
        .build_headers(&request.headers)
        .map_err(|e| Error::InvalidRequest(e.to_string()))?;

    let (body_bytes, default_content_type) = encode_request_body(
        method,
        prepared.endpoint.effective_request_body_type(),
        request.body.as_ref(),
    )?;

    let mut builder = client.request(as_reqwest_method(method), url);

    if let Some(timeout_ms) = prepared.endpoint.timeout_ms() {
        builder = builder.timeout(Duration::from_millis(timeout_ms));
    }

    for (name, value) in &layered_headers {
        builder = builder.header(name, value);
    }

    if let Some(content_type) = default_content_type
        && !has_content_type_header(&layered_headers)
    {
        builder = builder.header(reqwest::header::CONTENT_TYPE, content_type);
    }

    if let Some(bytes) = body_bytes {
        builder = builder.body(bytes);
    }

    let response = builder.send().await.map_err(|err| {
        if err.is_timeout() {
            Error::UpstreamTimeout
        } else {
            Error::UpstreamUnreachable(err.to_string())
        }
    })?;

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

fn as_reqwest_method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Head => reqwest::Method::HEAD,
        HttpMethod::Options => reqwest::Method::OPTIONS,
    }
}

fn has_content_type_header(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(reqwest::header::CONTENT_TYPE.as_str()))
}

fn retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers.get(reqwest::header::RETRY_AFTER).map(|value| {
        value
            .to_str()
            .map(str::to_owned)
            .unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).into_owned())
    })
}
