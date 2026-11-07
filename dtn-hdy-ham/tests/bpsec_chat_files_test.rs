use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use dtn_hdy_ham::chat_manager::ChatMessage;
use dtn_hdy_ham::config::HamConfig;
use dtn_hdy_ham::security::keystore::{Direction, KeyEntry, VerifyPolicy, sign_bundle_with_key};
use dtn_hdy_ham::server::HamServer;
use dtn_hdy_ham::web::{AppState, create_router};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::Eid;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn test_bpsec_chat_signing_and_verification_pipeline() {
    let config = HamConfig {
        callsign: Some("STATA".to_string()),
        node_id: "dtn://stata/".to_string(),
        storage: dtn_hdy_ham::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: dtn_hdy_ham::config::GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        web: Some(dtn_hdy_ham::config::WebConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            static_dir: std::path::PathBuf::from("web/dist"),
        }),
        ..Default::default()
    };

    let server = HamServer::new(config).await.unwrap();
    server.bpa().start(true).await;

    // Register ChatService with BPA
    let chat_service = Arc::new(dtn_hdy_ham::chat_service::ChatService::new(
        server.chat_manager(),
        server.service_tracker(),
        server.keystore(),
    ));
    server
        .service_tracker()
        .register_service_with_type(
            hardy_bpv7::eid::Service::Dtn("chat".into()),
            chat_service,
            "Built-in Chat",
        )
        .await
        .expect("chat service should register");

    let keystore = server.keystore();

    // 1. Configure Outbound key for F4JXQ
    keystore
        .add_key(
            Direction::Outbound,
            "dtn://f4jxq/*".to_string(),
            "outbound-secret-key-123".as_bytes().to_vec(),
            Some("F4JXQ Station".to_string()),
        )
        .unwrap();

    // Configure Inbound key for F4JXQ (different secret for inbound test)
    keystore
        .add_key(
            Direction::Inbound,
            "dtn://f4jxq/*".to_string(),
            "inbound-secret-key-456".as_bytes().to_vec(),
            Some("F4JXQ Peer Inbound".to_string()),
        )
        .unwrap();

    let injector = server.web_injector().expect("injector configured");
    let app_state = AppState {
        config: Arc::new(server.config().clone()),
        injector: injector.clone(),
        service_tracker: server.service_tracker(),
        cla_manager: server.cla_manager(),
        chat_manager: server.chat_manager(),
        internal_storage: server.internal_storage(),
        route_manager: server.route_manager(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
        keystore: server.keystore(),
    };
    let app = create_router(app_state);

    // 2. Dispatch outbound chat message to f4jxq: should be signed automatically and have auth_status = "valid"
    let send_body = serde_json::json!({
        "content": "Hello authenticated F4JXQ!"
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/chat/conversations/f4jxq")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&send_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let sent_msg: ChatMessage = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(sent_msg.auth_status, "valid");

    // 3. Inject inbound bundle signed with matching inbound key -> auth_status = "valid"
    let source_eid: Eid = "dtn://f4jxq/chat".parse().unwrap();
    let dest_eid: Eid = "dtn://stata/chat".parse().unwrap();
    let (raw_bundle, raw_bin) = Builder::new(source_eid.clone(), dest_eid.clone())
        .with_payload("Signed inbound message".as_bytes().into())
        .with_lifetime(Duration::from_secs(3600))
        .build(CreationTimestamp::now())
        .unwrap();

    let inbound_key_entry = KeyEntry {
        id: "test-inbound".to_string(),
        direction: Direction::Inbound,
        pattern: "dtn://f4jxq/*".to_string(),
        label: None,
        secret: "inbound-secret-key-456".as_bytes().to_vec(),
        created_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let signed_bin = sign_bundle_with_key(&raw_bundle, &raw_bin, &inbound_key_entry, None).unwrap();

    injector
        .inject(Bytes::from(signed_bin))
        .await
        .expect("injection should succeed");

    tokio::time::sleep(Duration::from_millis(150)).await;

    // Check conversation messages: the new inbound message should have auth_status = "valid"
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations/f4jxq")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let messages: Vec<ChatMessage> = serde_json::from_slice(&bytes).unwrap();
    let inbound_valid = messages
        .iter()
        .find(|m| m.content == "Signed inbound message")
        .expect("inbound message found");
    assert_eq!(inbound_valid.auth_status, "valid");

    // 4. Inject inbound bundle signed with WRONG key -> auth_status = "invalid"
    let (raw_bundle2, raw_bin2) = Builder::new(source_eid.clone(), dest_eid.clone())
        .with_payload("Tampered inbound message".as_bytes().into())
        .with_lifetime(Duration::from_secs(3600))
        .build(CreationTimestamp::now())
        .unwrap();

    let wrong_key_entry = KeyEntry {
        id: "wrong".to_string(),
        direction: Direction::Inbound,
        pattern: "dtn://f4jxq/*".to_string(),
        label: None,
        secret: "wrong-secret-key-999".as_bytes().to_vec(),
        created_at: "2026-11-07T00:00:00Z".to_string(),
    };
    let signed_bin2 =
        sign_bundle_with_key(&raw_bundle2, &raw_bin2, &wrong_key_entry, None).unwrap();

    injector
        .inject(Bytes::from(signed_bin2))
        .await
        .expect("injection should succeed");

    tokio::time::sleep(Duration::from_millis(150)).await;

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations/f4jxq")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let messages: Vec<ChatMessage> = serde_json::from_slice(&bytes).unwrap();
    let inbound_invalid = messages
        .iter()
        .find(|m| m.content == "Tampered inbound message")
        .expect("inbound invalid message found");
    assert_eq!(inbound_invalid.auth_status, "invalid");

    // 4b. Inject inbound bundle with identity mismatch: security_source is from dtn://attacker/ while primary is dtn://f4jxq/
    let (raw_bundle_mismatch, raw_bin_mismatch) =
        Builder::new(source_eid.clone(), dest_eid.clone())
            .with_payload("Spoofed source inbound message".as_bytes().into())
            .with_lifetime(Duration::from_secs(3600))
            .build(CreationTimestamp::now())
            .unwrap();

    let attacker_eid: Eid = "dtn://attacker/chat".parse().unwrap();
    let signed_bin_mismatch = sign_bundle_with_key(
        &raw_bundle_mismatch,
        &raw_bin_mismatch,
        &inbound_key_entry,
        Some(attacker_eid),
    )
    .unwrap();

    injector
        .inject(Bytes::from(signed_bin_mismatch))
        .await
        .expect("injection should succeed");

    tokio::time::sleep(Duration::from_millis(150)).await;

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations/f4jxq")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let messages: Vec<ChatMessage> = serde_json::from_slice(&bytes).unwrap();
    let inbound_mismatch = messages
        .iter()
        .find(|m| m.content == "Spoofed source inbound message")
        .expect("inbound mismatch message found");
    assert_eq!(inbound_mismatch.auth_status, "invalid");

    // 5. Inject unsigned bundle -> auth_status = "unsigned"
    let (_raw_bundle3, raw_bin3) = Builder::new(source_eid.clone(), dest_eid.clone())
        .with_payload("Unsigned inbound message".as_bytes().into())
        .with_lifetime(Duration::from_secs(3600))
        .build(CreationTimestamp::now())
        .unwrap();

    injector
        .inject(Bytes::from(raw_bin3.into_vec()))
        .await
        .expect("injection should succeed");

    tokio::time::sleep(Duration::from_millis(150)).await;

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations/f4jxq")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let messages: Vec<ChatMessage> = serde_json::from_slice(&bytes).unwrap();
    let inbound_unsigned = messages
        .iter()
        .find(|m| m.content == "Unsigned inbound message")
        .expect("inbound unsigned message found");
    assert_eq!(inbound_unsigned.auth_status, "unsigned");

    // 6. Test STRICT verification policy: invalid signatures get immediately dropped!
    keystore.set_policy(VerifyPolicy::Strict).unwrap();

    let (raw_bundle4, raw_bin4) = Builder::new(source_eid.clone(), dest_eid.clone())
        .with_payload("Strict dropped invalid message".as_bytes().into())
        .with_lifetime(Duration::from_secs(3600))
        .build(CreationTimestamp::now())
        .unwrap();

    let signed_bin4 =
        sign_bundle_with_key(&raw_bundle4, &raw_bin4, &wrong_key_entry, None).unwrap();

    injector
        .inject(Bytes::from(signed_bin4))
        .await
        .expect("injection should succeed");

    tokio::time::sleep(Duration::from_millis(150)).await;

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations/f4jxq")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let messages: Vec<ChatMessage> = serde_json::from_slice(&bytes).unwrap();
    assert!(
        messages
            .iter()
            .all(|m| m.content != "Strict dropped invalid message"),
        "Invalid message must be dropped under Strict policy"
    );
}
