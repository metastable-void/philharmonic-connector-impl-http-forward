use base64::Engine;
use philharmonic_connector_common::{UnixMillis, Uuid};
use philharmonic_connector_impl_api::{ConnectorCallContext, Implementation};
use philharmonic_connector_impl_http_forward::HttpForward;
use serde_json::json;
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

#[tokio::test]
async fn fixed_config_and_request_yield_expected_outbound_http_request() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/users/abc%2Fdef%20with%20space/events"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Request-Id", "req-vector")
                .set_body_json(json!({"ok": true})),
        )
        .mount(&server)
        .await;

    let config = json!({
        "endpoint": {
            "method": "post",
            "url_template": format!("{}/v1/users/{{user_id}}/events", server.uri()),
            "url_param_specs": {
                "user_id": {
                    "default": null,
                    "min_bytes": 1,
                    "max_bytes": 128
                }
            },
            "query_specs": [
                {"type": "const", "key": "api_version", "value": "2026-04"},
                {"type": "slotted", "key": "trace_id", "slot": "trace", "mode": "required"}
            ],
            "headers": {
                "Authorization": "Bearer baked",
                "X-Static": "one"
            },
            "overridable_request_headers": ["Idempotency-Key"],
            "exposed_response_headers": ["X-Request-Id"],
            "request_body_type": "bytes",
            "response_body_type": "json",
            "response_max_bytes": 1024,
            "timeout_ms": 5_000,
            "allow_non_2xx_status": false,
            "retry_policy": {
                "max_attempts": 1,
                "base_backoff_ms": 1,
                "max_backoff_ms": 1,
                "max_retry_delay_ms": 1_000,
                "rate_limit_backoff_ms": 1,
                "retry_on_io_errors": true,
                "retry_on_timeout": true,
                "respect_retry_after": true,
                "retry_on_status": [500]
            }
        }
    });

    let payload_bytes = vec![0_u8, 10, 20, 30, 40, 250];
    let request = json!({
        "urlParams": {"user_id": "abc/def with space"},
        "queries": {"trace": "trace-123"},
        "headers": {"Idempotency-Key": "idem-1"},
        "body": base64::engine::general_purpose::STANDARD.encode(&payload_bytes)
    });

    let impl_ = HttpForward::new().unwrap();
    let result = impl_
        .execute(&config, &request, &call_context())
        .await
        .unwrap();

    assert_eq!(result["status"], json!(200));
    assert_eq!(result["headers"]["x-request-id"], json!("req-vector"));

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);

    let outbound = &received[0];
    assert_eq!(outbound.method.as_str(), "POST");
    assert_eq!(
        outbound.url.path(),
        "/v1/users/abc%2Fdef%20with%20space/events"
    );

    let query_pairs = outbound
        .url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<Vec<_>>();
    assert!(query_pairs.contains(&("api_version".to_owned(), "2026-04".to_owned())));
    assert!(query_pairs.contains(&("trace_id".to_owned(), "trace-123".to_owned())));

    assert_eq!(
        outbound
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer baked"
    );
    assert_eq!(
        outbound.headers.get("x-static").unwrap().to_str().unwrap(),
        "one"
    );
    assert_eq!(
        outbound
            .headers
            .get("idempotency-key")
            .unwrap()
            .to_str()
            .unwrap(),
        "idem-1"
    );
    assert_eq!(
        outbound
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/octet-stream"
    );

    assert_eq!(outbound.body, payload_bytes);
}
