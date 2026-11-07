use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use dtn_hdy_ham::chat_manager::{ChatMessage, ConversationSummary};
use dtn_hdy_ham::config::HamConfig;
use dtn_hdy_ham::server::HamServer;
use dtn_hdy_ham::web::{AppState, create_router};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::Eid;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn test_chat_service_full_workflow() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug,hardy_bpa=trace,dtn_hdy_ham=trace")
        .try_init();

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

    let server = HamServer::new(config.clone())
        .await
        .expect("HamServer should build");

    server.bpa().start(true).await;

    // Register built-in Chat service as HamServer::run does
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

    let injector = server.web_injector().expect("injector configured");
    let service_tracker = server.service_tracker();
    let cla_manager = server.cla_manager();
    let chat_manager = server.chat_manager();
    let internal_storage = server.internal_storage();

    let state = AppState {
        config: Arc::new(server.config().clone()),
        injector: injector.clone(),
        service_tracker: service_tracker.clone(),
        cla_manager,
        chat_manager: chat_manager.clone(),
        internal_storage,
        route_manager: server.route_manager(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
        keystore: server.keystore(),
    };
    let app = create_router(state);

    // 1. Initial list of conversations should be empty
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let convs: Vec<ConversationSummary> = serde_json::from_slice(&bytes).unwrap();
    assert!(convs.is_empty());

    // 2. Dispatch outbound chat message to f4jxq
    let send_body = serde_json::json!({
        "content": "Hello F4JXQ de STATA!"
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
    assert_eq!(sent_msg.conversation_peer, "dtn://f4jxq/chat");
    assert_eq!(sent_msg.sender_eid, "dtn://stata/chat");
    assert_eq!(sent_msg.recipient_eid, "dtn://f4jxq/chat");
    assert_eq!(sent_msg.content, "Hello F4JXQ de STATA!");
    assert_eq!(sent_msg.direction, "outbound");
    assert_eq!(sent_msg.status, "sent");

    // 3. Verify conversation list has 1 entry
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let convs: Vec<ConversationSummary> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0].peer, "dtn://f4jxq/chat");
    assert_eq!(convs[0].unread_count, 0);
    assert_eq!(convs[0].total_messages, 1);

    // 4. Test delivering an inbound bundle to dtn://stata/chat without CRC (CrcType::None)
    // To verify that primary_block_integrity = false accepts bundles without CRC/BIB from peers like f4jxq-9
    let source_eid: Eid = "dtn://f4jxq/chat".parse().unwrap();
    let dest_eid: Eid = "dtn://stata/chat".parse().unwrap();
    let (_inbound_bundle, inbound_bin) = Builder::new(source_eid, dest_eid)
        .with_crc_type(hardy_bpv7::crc::CrcType::None)
        .with_payload("73 de F4JXQ! All copy 59.".as_bytes().into())
        .with_lifetime(Duration::from_secs(3600))
        .build(CreationTimestamp::now())
        .expect("inbound bundle should build");

    injector
        .inject(Bytes::from(inbound_bin.into_vec()))
        .await
        .expect("injection should succeed");

    // Give BPA engine time to deliver to ChatService
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 5. Check conversations list: should show 1 unread message
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let convs: Vec<ConversationSummary> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0].unread_count, 1);
    assert_eq!(convs[0].total_messages, 2);
    assert_eq!(convs[0].last_message, "73 de F4JXQ! All copy 59.");
    assert_eq!(convs[0].last_direction, "inbound");

    // 6. Retrieve message history via query endpoint /chat/messages?peer=f4jxq
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/messages?peer=f4jxq")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let msgs: Vec<ChatMessage> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "Hello F4JXQ de STATA!");
    assert_eq!(msgs[1].content, "73 de F4JXQ! All copy 59.");
    assert_eq!(msgs[1].direction, "inbound");
    assert_eq!(msgs[1].status, "received");
    assert!(!msgs[1].is_read);

    // 7. Mark conversation read
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/chat/conversations/f4jxq/read")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let read_res: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(read_res["status"], "ok");
    assert_eq!(read_res["marked"], 1);

    // 8. Verify unread_count is now 0
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let convs: Vec<ConversationSummary> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(convs[0].unread_count, 0);

    // 9. Explicitly create a new conversation thread via POST /api/v1/chat/conversations
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/chat/conversations")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"peer": "dtn://f4jxq-9/chat"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let new_conv: ConversationSummary = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(new_conv.peer, "dtn://f4jxq-9/chat");
    assert_eq!(new_conv.total_messages, 0);

    // 10. Also add a stats service chat: dtn://f4jxq/stats
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/chat/conversations")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"peer": "dtn://f4jxq/stats"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // Verify all 3 distinct chats now exist in conversation list
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chat/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let all_convs: Vec<ConversationSummary> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(all_convs.len(), 3);
    let peers: Vec<String> = all_convs.into_iter().map(|c| c.peer).collect();
    assert!(peers.contains(&"dtn://f4jxq/chat".to_string()));
    assert!(peers.contains(&"dtn://f4jxq-9/chat".to_string()));
    assert!(peers.contains(&"dtn://f4jxq/stats".to_string()));

    // 11. Verify message timestamps are in Unix epoch ms and survive expiration purge
    let msgs = chat_manager
        .get_conversation_messages("dtn://f4jxq/chat")
        .unwrap();
    assert_eq!(msgs.len(), 2);
    for m in &msgs {
        assert!(
            m.timestamp > 1_700_000_000_000,
            "Timestamp {} must be in Unix epoch ms (> 1.7*10^12)",
            m.timestamp
        );
        assert!(
            m.expiry_timestamp > m.timestamp,
            "Expiry timestamp {} must be greater than timestamp {}",
            m.expiry_timestamp,
            m.timestamp
        );
        // Both outbound and inbound messages must have 3600s (1h) lifetime
        assert_eq!(
            m.expiry_timestamp - m.timestamp,
            3_600_000,
            "Message lifetime must be 3600s (3600000 ms)"
        );
    }

    let now_ms = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
    let purged = chat_manager.purge_expired(now_ms).unwrap();
    assert_eq!(purged, 0, "Non-expired messages must not be purged");

    let msgs_after = chat_manager
        .get_conversation_messages("dtn://f4jxq/chat")
        .unwrap();
    assert_eq!(
        msgs_after.len(),
        2,
        "Messages must persist across expiration checks"
    );

    // 12. Advance time past 1 hour: both outbound and inbound messages must be expired and purged
    let future_ms = now_ms + 3_601_000;
    let purged = chat_manager.purge_expired(future_ms).unwrap();
    assert_eq!(
        purged, 2,
        "Both outbound and inbound chat messages must be purged after 1 hour"
    );

    let msgs_expired = chat_manager
        .get_conversation_messages("dtn://f4jxq/chat")
        .unwrap();
    assert_eq!(
        msgs_expired.len(),
        0,
        "No messages should remain in conversation after expiration"
    );

    // Conversation thread must still be preserved in conversations list
    let convs = chat_manager.list_conversations().unwrap();
    let thread = convs.iter().find(|c| c.peer == "dtn://f4jxq/chat");
    assert!(thread.is_some(), "Conversation thread should still persist");
    assert_eq!(thread.unwrap().total_messages, 0);
}
