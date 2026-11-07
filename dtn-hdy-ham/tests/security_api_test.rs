use axum::body::Body;
use axum::http::{Request, StatusCode};
use dtn_hdy_ham::config::HamConfig;
use dtn_hdy_ham::server::HamServer;
use dtn_hdy_ham::web::{AppState, create_router};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn test_security_policy_and_keys_api() {
    let config = HamConfig {
        callsign: Some("F5KUV".to_string()),
        node_id: "dtn://f5kuv/".to_string(),
        storage: dtn_hdy_ham::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        web: Some(dtn_hdy_ham::config::WebConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            static_dir: std::path::PathBuf::from("web/dist"),
        }),
        ..Default::default()
    };
    let server = HamServer::new(config).await.unwrap();
    let injector = server.web_injector().expect("injector configured");
    let app_state = AppState {
        config: Arc::new(server.config().clone()),
        injector,
        service_tracker: server.service_tracker(),
        cla_manager: server.cla_manager(),
        chat_manager: server.chat_manager(),
        internal_storage: server.internal_storage(),
        route_manager: server.route_manager(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
        keystore: server.keystore(),
        senml_manager: server.senml_manager(),
    };
    let app = create_router(app_state);

    // 1. GET /api/v1/security/policy (default is warn)
    let req = Request::builder()
        .uri("/api/v1/security/policy")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["policy"], "warn");

    // 2. PUT /api/v1/security/policy -> strict
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/security/policy")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"policy": "strict"}"#))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Verify it updated
    let req = Request::builder()
        .uri("/api/v1/security/policy")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["policy"], "strict");

    // 3. GET /api/v1/security/keys (should be empty initially)
    let req = Request::builder()
        .uri("/api/v1/security/keys")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let keys: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(keys.as_array().unwrap().len(), 0);

    // 3b. Try to add a key with secret < 16 bytes: should return 400 Bad Request
    let short_payload = serde_json::json!({
        "direction": "outbound",
        "pattern": "dtn://f4jxq/*",
        "label": "Short Secret",
        "secret": "too-short"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/security/keys")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&short_payload).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 4. POST /api/v1/security/keys -> add outbound key
    let add_payload = serde_json::json!({
        "direction": "outbound",
        "pattern": "dtn://f4jxq/*",
        "label": "F4JXQ Outbound",
        "secret": "my-secret-key-phrase"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/security/keys")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&add_payload).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&body).unwrap();
    let key_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["direction"], "outbound");
    assert_eq!(created["pattern"], "dtn://f4jxq/*");
    assert_eq!(created["label"], "F4JXQ Outbound");
    assert!(created["fingerprint"].as_str().is_some());
    // Secret should never be leaked in response!
    assert!(created.get("secret").is_none());

    // 5. POST /api/v1/security/keys -> add inbound key
    let add_inbound = serde_json::json!({
        "direction": "inbound",
        "pattern": "dtn://f4jxq/*",
        "label": "F4JXQ Inbound",
        "secret": "another-secret-phrase"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/security/keys")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&add_inbound).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // 6. List keys again - should have 2 keys
    let req = Request::builder()
        .uri("/api/v1/security/keys")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let keys: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(keys.as_array().unwrap().len(), 2);

    // 7. DELETE /api/v1/security/keys/{id}
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/security/keys/{}", key_id))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 8. List keys again - should have 1 key
    let req = Request::builder()
        .uri("/api/v1/security/keys")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let keys: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(keys.as_array().unwrap().len(), 1);
}
