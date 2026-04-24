use philharmonic_connector_common::{UnixMillis, Uuid};
use philharmonic_connector_impl_api::{
    ConnectorCallContext, Implementation, ImplementationError, JsonValue,
};
use philharmonic_connector_impl_http_forward::HttpForward;
use serde_json::json;
use std::{net::TcpListener, time::Duration};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
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

fn endpoint_config(
    url_template: String,
    allow_non_2xx_status: bool,
    response_body_type: &str,
    response_max_bytes: Option<usize>,
    timeout_ms: u64,
) -> JsonValue {
    json!({
        "endpoint": {
            "method": "get",
            "url_template": url_template,
            "url_param_specs": {},
            "query_specs": [],
            "headers": {},
            "overridable_request_headers": [],
            "exposed_response_headers": ["X-Request-Id"],
            "request_body_type": "json",
            "response_body_type": response_body_type,
            "response_max_bytes": response_max_bytes,
            "timeout_ms": timeout_ms,
            "allow_non_2xx_status": allow_non_2xx_status,
            "retry_policy": {
                "max_attempts": 1,
                "base_backoff_ms": 1,
                "max_backoff_ms": 1,
                "max_retry_delay_ms": 1_000,
                "rate_limit_backoff_ms": 1,
                "retry_on_io_errors": true,
                "retry_on_timeout": true,
                "respect_retry_after": true,
                "retry_on_status": [429, 500]
            }
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

#[tokio::test]
async fn invalid_config_when_response_max_bytes_missing() {
    let server = MockServer::start().await;
    let cfg = endpoint_config(
        format!("{}/missing", server.uri()),
        false,
        "json",
        None,
        500,
    );

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(&cfg, &request(), &call_context())
        .await
        .unwrap_err();

    assert_eq!(
        err,
        ImplementationError::InvalidConfig {
            detail: "missing response_max_bytes".to_owned(),
        }
    );
}

#[tokio::test]
async fn invalid_request_on_unknown_header_override() {
    let server = MockServer::start().await;
    let cfg = endpoint_config(
        format!("{}/bad-header", server.uri()),
        false,
        "json",
        Some(1024),
        500,
    );

    let bad_request = json!({
        "urlParams": {},
        "queries": {},
        "headers": {"X-Not-Allowlisted": "value"},
        "body": null
    });

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(&cfg, &bad_request, &call_context())
        .await
        .unwrap_err();

    match err {
        ImplementationError::InvalidRequest { detail } => {
            assert!(detail.contains("not allowlisted"));
        }
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
}

#[tokio::test]
async fn upstream_error_payload_for_non_success() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/not-found"))
        .respond_with(
            ResponseTemplate::new(404)
                .insert_header("X-Request-Id", "req_404")
                .set_body_json(json!({"message": "missing"})),
        )
        .mount(&server)
        .await;

    let cfg = endpoint_config(
        format!("{}/not-found", server.uri()),
        false,
        "json",
        Some(1024),
        500,
    );

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(&cfg, &request(), &call_context())
        .await
        .unwrap_err();

    let ImplementationError::UpstreamError { status, body } = err else {
        panic!("expected UpstreamError");
    };

    assert_eq!(status, 404);
    let payload: JsonValue = serde_json::from_str(&body).unwrap();
    assert_eq!(payload["status"], json!(404));
    assert_eq!(payload["headers"]["x-request-id"], json!("req_404"));
    assert_eq!(payload["body"]["message"], json!("missing"));
}

#[tokio::test]
async fn non_success_can_be_returned_when_allowed() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/allowed"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"ok": false})))
        .mount(&server)
        .await;

    let cfg = endpoint_config(
        format!("{}/allowed", server.uri()),
        true,
        "json",
        Some(1024),
        500,
    );

    let impl_ = HttpForward::new().unwrap();
    let out = impl_
        .execute(&cfg, &request(), &call_context())
        .await
        .unwrap();

    assert_eq!(out["status"], json!(404));
    assert_eq!(out["ok"], json!(false));
    assert_eq!(out["body"]["ok"], json!(false));
}

#[tokio::test]
async fn upstream_unreachable_when_connection_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let cfg = endpoint_config(
        format!("http://{addr}/refused"),
        false,
        "json",
        Some(1024),
        100,
    );

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(&cfg, &request(), &call_context())
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ImplementationError::UpstreamUnreachable { .. }
    ));
}

#[tokio::test]
async fn upstream_timeout_when_attempt_times_out() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
        .mount(&server)
        .await;

    let cfg = endpoint_config(
        format!("{}/slow", server.uri()),
        false,
        "json",
        Some(1024),
        50,
    );

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(&cfg, &request(), &call_context())
        .await
        .unwrap_err();

    assert_eq!(err, ImplementationError::UpstreamTimeout);
}

#[tokio::test]
async fn response_too_large_is_reported() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string("abcdefghijklmnopqrstuvwxyz"))
        .mount(&server)
        .await;

    let cfg = endpoint_config(format!("{}/big", server.uri()), false, "utf8", Some(5), 500);

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(&cfg, &request(), &call_context())
        .await
        .unwrap_err();

    match err {
        ImplementationError::ResponseTooLarge { limit, actual } => {
            assert_eq!(limit, 5);
            assert!(actual > 5);
        }
        other => panic!("expected ResponseTooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn invalid_json_response_maps_to_internal() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/invalid-json"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not-json"))
        .mount(&server)
        .await;

    let cfg = endpoint_config(
        format!("{}/invalid-json", server.uri()),
        false,
        "json",
        Some(1024),
        500,
    );

    let impl_ = HttpForward::new().unwrap();
    let err = impl_
        .execute(&cfg, &request(), &call_context())
        .await
        .unwrap_err();

    match err {
        ImplementationError::Internal { detail } => {
            assert!(detail.contains("decode json response body"));
        }
        other => panic!("expected Internal, got {other:?}"),
    }
}
