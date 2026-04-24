use philharmonic_connector_common::{UnixMillis, Uuid};
use philharmonic_connector_impl_api::{
    ConnectorCallContext, Implementation, ImplementationError, JsonValue,
};
use philharmonic_connector_impl_http_forward::HttpForward;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

fn call_context() -> ConnectorCallContext {
    ConnectorCallContext {
        tenant_id: Uuid::nil(),
        instance_id: Uuid::nil(),
        step_seq: 1,
        config_uuid: Uuid::nil(),
        issued_at: UnixMillis(0),
        expires_at: UnixMillis(10_000),
    }
}

fn config(url_template: String, retry_policy: JsonValue) -> JsonValue {
    json!({
        "endpoint": {
            "method": "get",
            "url_template": url_template,
            "url_param_specs": {},
            "query_specs": [],
            "headers": {},
            "overridable_request_headers": [],
            "exposed_response_headers": ["Retry-After"],
            "request_body_type": "json",
            "response_body_type": "json",
            "response_max_bytes": 1024,
            "timeout_ms": 2_000,
            "allow_non_2xx_status": false,
            "retry_policy": retry_policy
        }
    })
}

fn request() -> JsonValue {
    json!({
        "urlParams": {},
        "queries": {},
        "headers": {},
        "body": null
    })
}

#[derive(Clone)]
struct SequenceResponder {
    templates: Arc<Vec<ResponseTemplate>>,
    calls: Arc<AtomicUsize>,
}

impl Respond for SequenceResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let idx = self.calls.fetch_add(1, Ordering::SeqCst);
        match self.templates.get(idx) {
            Some(template) => template.clone(),
            None => self
                .templates
                .last()
                .cloned()
                .unwrap_or_else(|| ResponseTemplate::new(500)),
        }
    }
}

#[tokio::test]
async fn retries_5xx_then_succeeds() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/retry-5xx"))
        .respond_with(SequenceResponder {
            templates: Arc::new(vec![
                ResponseTemplate::new(500).set_body_json(json!({"error": "temporary"})),
                ResponseTemplate::new(200).set_body_json(json!({"ok": true})),
            ]),
            calls: calls.clone(),
        })
        .mount(&server)
        .await;

    let retry_policy = json!({
        "max_attempts": 3,
        "base_backoff_ms": 1,
        "max_backoff_ms": 2,
        "max_retry_delay_ms": 1_000,
        "rate_limit_backoff_ms": 1,
        "retry_on_io_errors": true,
        "retry_on_timeout": true,
        "respect_retry_after": true,
        "retry_on_status": [500]
    });

    let impl_ = HttpForward::new().unwrap();
    let out = impl_
        .execute(
            &config(format!("{}/retry-5xx", server.uri()), retry_policy),
            &request(),
            &call_context(),
        )
        .await
        .unwrap();

    assert_eq!(out["status"], json!(200));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retries_429_with_retry_after_header() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/retry-429-header"))
        .respond_with(SequenceResponder {
            templates: Arc::new(vec![
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "0")
                    .set_body_json(json!({"error": "rate limited"})),
                ResponseTemplate::new(200).set_body_json(json!({"ok": true})),
            ]),
            calls: calls.clone(),
        })
        .mount(&server)
        .await;

    let retry_policy = json!({
        "max_attempts": 3,
        "base_backoff_ms": 1,
        "max_backoff_ms": 2,
        "max_retry_delay_ms": 1_000,
        "rate_limit_backoff_ms": 50,
        "retry_on_io_errors": true,
        "retry_on_timeout": true,
        "respect_retry_after": true,
        "retry_on_status": [429]
    });

    let impl_ = HttpForward::new().unwrap();
    let out = impl_
        .execute(
            &config(format!("{}/retry-429-header", server.uri()), retry_policy),
            &request(),
            &call_context(),
        )
        .await
        .unwrap();

    assert_eq!(out["status"], json!(200));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retries_429_without_retry_after_header() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/retry-429-no-header"))
        .respond_with(SequenceResponder {
            templates: Arc::new(vec![
                ResponseTemplate::new(429).set_body_json(json!({"error": "rate limited"})),
                ResponseTemplate::new(200).set_body_json(json!({"ok": true})),
            ]),
            calls: calls.clone(),
        })
        .mount(&server)
        .await;

    let retry_policy = json!({
        "max_attempts": 3,
        "base_backoff_ms": 1,
        "max_backoff_ms": 2,
        "max_retry_delay_ms": 1_000,
        "rate_limit_backoff_ms": 1,
        "retry_on_io_errors": true,
        "retry_on_timeout": true,
        "respect_retry_after": true,
        "retry_on_status": [429]
    });

    let impl_ = HttpForward::new().unwrap();
    let out = impl_
        .execute(
            &config(
                format!("{}/retry-429-no-header", server.uri()),
                retry_policy,
            ),
            &request(),
            &call_context(),
        )
        .await
        .unwrap();

    assert_eq!(out["status"], json!(200));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn max_attempts_exhausted_returns_last_error() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/retry-exhausted"))
        .respond_with(SequenceResponder {
            templates: Arc::new(vec![ResponseTemplate::new(500).set_body_json(json!({
                "error": "still failing"
            }))]),
            calls: calls.clone(),
        })
        .mount(&server)
        .await;

    let retry_policy = json!({
        "max_attempts": 2,
        "base_backoff_ms": 1,
        "max_backoff_ms": 1,
        "max_retry_delay_ms": 1_000,
        "rate_limit_backoff_ms": 1,
        "retry_on_io_errors": true,
        "retry_on_timeout": true,
        "respect_retry_after": true,
        "retry_on_status": [500]
    });

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(
            &config(format!("{}/retry-exhausted", server.uri()), retry_policy),
            &request(),
            &call_context(),
        )
        .await
        .unwrap_err();

    let ImplementationError::UpstreamError { status, .. } = err else {
        panic!("expected UpstreamError");
    };
    assert_eq!(status, 500);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn overall_deadline_returns_upstream_timeout() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/deadline"))
        .respond_with(SequenceResponder {
            templates: Arc::new(vec![ResponseTemplate::new(429)]),
            calls: calls.clone(),
        })
        .mount(&server)
        .await;

    let retry_policy = json!({
        "max_attempts": 3,
        "base_backoff_ms": 1,
        "max_backoff_ms": 1,
        "max_retry_delay_ms": 10,
        "rate_limit_backoff_ms": 10,
        "retry_on_io_errors": true,
        "retry_on_timeout": true,
        "respect_retry_after": false,
        "retry_on_status": [429]
    });

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(
            &config(format!("{}/deadline", server.uri()), retry_policy),
            &request(),
            &call_context(),
        )
        .await
        .unwrap_err();

    assert_eq!(err, ImplementationError::UpstreamTimeout);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
