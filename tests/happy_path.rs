use base64::Engine;
use philharmonic_connector_common::{UnixMillis, Uuid};
use philharmonic_connector_impl_api::{ConnectorCallContext, Implementation};
use philharmonic_connector_impl_http_forward::HttpForward;
use serde_json::{Value as JsonValue, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
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

fn config(server: &MockServer, request_body_type: &str, response_body_type: &str) -> JsonValue {
    json!({
        "endpoint": {
            "method": "post",
            "url_template": format!("{}/v1/items/{{item_id}}", server.uri()),
            "url_param_specs": {
                "item_id": {
                    "default": null,
                    "min_bytes": 1,
                    "max_bytes": 64
                }
            },
            "query_specs": [
                { "type": "const", "key": "api_version", "value": "2026-01" },
                { "type": "slotted", "key": "trace", "slot": "trace", "mode": "optional" }
            ],
            "headers": {
                "Authorization": "Bearer secret"
            },
            "overridable_request_headers": ["Idempotency-Key"],
            "exposed_response_headers": ["X-Request-Id", "Content-Type"],
            "request_body_type": request_body_type,
            "response_body_type": response_body_type,
            "response_max_bytes": 8192,
            "timeout_ms": 5_000,
            "allow_non_2xx_status": false,
            "retry_policy": {
                "max_attempts": 1,
                "base_backoff_ms": 1,
                "max_backoff_ms": 1,
                "max_retry_delay_ms": 5_000,
                "rate_limit_backoff_ms": 1,
                "retry_on_io_errors": true,
                "retry_on_timeout": true,
                "respect_retry_after": true,
                "retry_on_status": [500]
            }
        }
    })
}

#[tokio::test]
async fn json_round_trip() {
    let server = MockServer::start().await;
    let template = ResponseTemplate::new(200)
        .insert_header("X-Request-Id", "r_json")
        .set_body_json(json!({"ok": true, "mode": "json"}));

    Mock::given(method("POST"))
        .and(path("/v1/items/item_1"))
        .and(query_param("api_version", "2026-01"))
        .and(query_param("trace", "trace_1"))
        .and(header("authorization", "Bearer secret"))
        .respond_with(template)
        .mount(&server)
        .await;

    let request = json!({
        "urlParams": {"item_id": "item_1"},
        "queries": {"trace": "trace_1"},
        "headers": {"Idempotency-Key": "id_1"},
        "body": {"hello": "world"}
    });

    let impl_ = HttpForward::new().unwrap();
    let out = impl_
        .execute(&config(&server, "json", "json"), &request, &call_context())
        .await
        .unwrap();

    assert_eq!(out["status"], json!(200));
    assert_eq!(out["ok"], json!(true));
    assert_eq!(out["headers"]["x-request-id"], json!("r_json"));
    assert_eq!(out["body"]["ok"], json!(true));
}

#[tokio::test]
async fn utf8_round_trip() {
    let server = MockServer::start().await;
    let template = ResponseTemplate::new(200)
        .insert_header("X-Request-Id", "r_utf8")
        .set_body_string("pong");

    Mock::given(method("POST"))
        .and(path("/v1/items/item_utf8"))
        .respond_with(template)
        .mount(&server)
        .await;

    let request = json!({
        "urlParams": {"item_id": "item_utf8"},
        "queries": {},
        "headers": {},
        "body": "ping"
    });

    let impl_ = HttpForward::new().unwrap();
    let out = impl_
        .execute(&config(&server, "utf8", "utf8"), &request, &call_context())
        .await
        .unwrap();

    assert_eq!(out["status"], json!(200));
    assert_eq!(out["body"], json!("pong"));
}

#[tokio::test]
async fn bytes_round_trip() {
    let server = MockServer::start().await;
    let raw = vec![0_u8, 1, 2, 3, 255];
    let template = ResponseTemplate::new(200)
        .insert_header("X-Request-Id", "r_bytes")
        .set_body_raw(raw.clone(), "application/octet-stream");

    Mock::given(method("POST"))
        .and(path("/v1/items/item_bytes"))
        .respond_with(template)
        .mount(&server)
        .await;

    let encoded_request = base64::engine::general_purpose::STANDARD.encode([5_u8, 4, 3, 2, 1]);
    let request = json!({
        "urlParams": {"item_id": "item_bytes"},
        "queries": {},
        "headers": {},
        "body": encoded_request
    });

    let impl_ = HttpForward::new().unwrap();
    let out = impl_
        .execute(
            &config(&server, "bytes", "bytes"),
            &request,
            &call_context(),
        )
        .await
        .unwrap();

    assert_eq!(out["status"], json!(200));
    let expected = base64::engine::general_purpose::STANDARD.encode(raw);
    assert_eq!(out["body"], json!(expected));
}
