use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use dtn_hdy_ham::config::HamConfig;
use dtn_hdy_ham::internal_storage::FileBundleRecord;
use dtn_hdy_ham::server::HamServer;
use dtn_hdy_ham::web::{AppState, create_router};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::Eid;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn test_files_service_workflow_and_degraded_cases() {
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

    // Register permanent built-in Files service
    let files_service = Arc::new(dtn_hdy_ham::files_service::FilesService::new(
        server.internal_storage(),
        server.service_tracker(),
        server.keystore(),
    ));
    server
        .service_tracker()
        .register_service_with_type(
            hardy_bpv7::eid::Service::Dtn("files".into()),
            files_service,
            "Built-in Files",
        )
        .await
        .expect("files service should register");

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
        chat_manager,
        internal_storage: internal_storage.clone(),
        route_manager: server.route_manager(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
        keystore: server.keystore(),
        senml_manager: server.senml_manager(),
    };
    let app = create_router(state);

    // 1. Initial GET /api/v1/files should be empty
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list: Vec<FileBundleRecord> = serde_json::from_slice(&bytes).unwrap();
    assert!(list.is_empty());

    // 2. Deliver inbound bundle to dtn://stata/files with Markdown + Data-URI image
    let sample_markdown = "# Station Notice 📡\nHere is a packet: ![pixel](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==)\n73 de F4JXQ!";
    let source_eid: Eid = "dtn://f4jxq/files".parse().unwrap();
    let dest_eid: Eid = "dtn://stata/files".parse().unwrap();
    let (_bundle, bin) = Builder::new(source_eid, dest_eid)
        .with_payload(sample_markdown.as_bytes().into())
        .with_lifetime(Duration::from_secs(7200))
        .build(CreationTimestamp::now())
        .expect("bundle builds");

    injector
        .inject(Bytes::from(bin.into_vec()))
        .await
        .expect("injection succeeds");

    // Wait for delivery to FilesService
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 3. Verify files list now contains 1 item
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list: Vec<FileBundleRecord> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.len(), 1);
    let bundle_id = list[0].id.clone();
    assert_eq!(list[0].source_eid, "dtn://f4jxq/files");
    assert_eq!(list[0].destination_eid, "dtn://stata/files");
    assert_eq!(list[0].mime_type, "text/markdown");
    assert!(!list[0].is_sticky);
    assert!(!list[0].is_read);

    // 4. Check unread count
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/unread")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let unread_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(unread_json["unread"], 1);

    // 5. Inspect file details (marks as read)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/files/{}", url_encode(&bundle_id)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let detail_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(detail_json["bundle"]["id"], bundle_id);
    assert_eq!(detail_json["bundle"]["is_read"], true);
    assert_eq!(detail_json["file_missing"], false);
    let preview = detail_json["payload_preview"].as_str().unwrap();
    assert!(preview.contains("Station Notice"));
    assert!(preview.contains("data:image/png;base64"));

    // 6. Stream/Download raw payload (inline by default with range support)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/files/{}/payload", url_encode(&bundle_id)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("content-type").unwrap(), "text/markdown");
    assert!(
        res.headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("inline")
    );
    assert_eq!(res.headers().get("accept-ranges").unwrap(), "bytes");
    let payload_bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&payload_bytes), sample_markdown);

    // 6b. Download with ?download=true returns attachment disposition
    let res_dl = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/files/{}/payload?download=true",
                    url_encode(&bundle_id)
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_dl.status(), StatusCode::OK);
    assert!(
        res_dl
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("attachment")
    );

    // 6c. Range request returns 206 Partial Content
    let res_range = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/files/{}/payload", url_encode(&bundle_id)))
                .header("Range", "bytes=0-9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res_range
            .headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap(),
        &format!("bytes 0-9/{}", sample_markdown.len())
    );
    let range_bytes = axum::body::to_bytes(res_range.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(range_bytes.len(), 10);
    assert_eq!(&range_bytes[..], &sample_markdown.as_bytes()[0..10]);

    // 7. Toggle sticky status
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/files/{}/sticky", url_encode(&bundle_id)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let sticky_json: Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(sticky_json["is_sticky"], true);

    // 8. Degraded cases:
    // 8a. Non-existent ID returns 404
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/non-existent-bundle-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/non-existent-bundle-id/payload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/files/non-existent-bundle-id/sticky")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 8b. Physical file deleted on disk -> API reports file_missing: true and payload download 404
    let file_rec = internal_storage
        .get_file_bundle(&bundle_id)
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_file(&file_rec.payload_path);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/files/{}", url_encode(&bundle_id)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let detail_json: Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(detail_json["file_missing"], true);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/files/{}/payload", url_encode(&bundle_id)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 8c. Deletion succeeds even when physical payload was already deleted
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/files/{}", url_encode(&bundle_id)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        internal_storage
            .get_file_bundle(&bundle_id)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_files_payload_range_requests_and_mime_detection() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage_dir = tmp_dir.path().to_path_buf();
    let internal_storage =
        dtn_hdy_ham::internal_storage::InternalStorage::new_disk(&storage_dir).unwrap();

    // Create an Opus audio test file: 50 bytes total with OggS and OpusHead headers
    let mut ogg_opus_payload = vec![0u8; 50];
    ogg_opus_payload[0..4].copy_from_slice(b"OggS");
    ogg_opus_payload[26] = 1; // page_segments = 1
    ogg_opus_payload[28..36].copy_from_slice(b"OpusHead");
    // Write unique byte indices at the end so we can verify slice offsets
    for (idx, slot) in ogg_opus_payload[40..50].iter_mut().enumerate() {
        *slot = (40 + idx) as u8;
    }

    let payload_path = storage_dir.join("test_audio.ogg");
    tokio::fs::write(&payload_path, &ogg_opus_payload)
        .await
        .unwrap();

    let record = FileBundleRecord {
        id: "test_audio_bundle".to_string(),
        source_eid: "dtn://sender/files".to_string(),
        destination_eid: "dtn://stata/files".to_string(),
        creation_timestamp: 12345,
        lifetime_secs: 3600,
        size: 50,
        mime_type: "audio/ogg".to_string(),
        payload_path: payload_path.to_str().unwrap().to_string(),
        created_at: "2026-11-05T10:00:00Z".to_string(),
        expiry_timestamp: 12345 + 3600 * 1000,
        is_sticky: false,
        is_read: false,
        auth_status: "unsigned".to_string(),
    };
    internal_storage.insert_file_bundle(&record).unwrap();

    // Create an MP4 video test file
    let mut mp4_payload = vec![0u8; 64];
    mp4_payload[0..4].copy_from_slice(b"\x00\x00\x00\x20");
    mp4_payload[4..8].copy_from_slice(b"ftyp");
    mp4_payload[8..12].copy_from_slice(b"isom");
    let video_path = storage_dir.join("test_video.mp4");
    tokio::fs::write(&video_path, &mp4_payload).await.unwrap();

    let video_record = FileBundleRecord {
        id: "test_video_bundle".to_string(),
        source_eid: "dtn://sender/files".to_string(),
        destination_eid: "dtn://stata/files".to_string(),
        creation_timestamp: 12346,
        lifetime_secs: 3600,
        size: 64,
        mime_type: "video/mp4".to_string(),
        payload_path: video_path.to_str().unwrap().to_string(),
        created_at: "2026-11-05T10:00:01Z".to_string(),
        expiry_timestamp: 12346 + 3600 * 1000,
        is_sticky: false,
        is_read: false,
        auth_status: "unsigned".to_string(),
    };
    internal_storage.insert_file_bundle(&video_record).unwrap();

    let config = HamConfig {
        callsign: Some("STATA".to_string()),
        node_id: "dtn://stata/".to_string(),
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
        internal_storage,
        route_manager: server.route_manager(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
        keystore: server.keystore(),
        senml_manager: server.senml_manager(),
    };
    let app = create_router(app_state);

    // 1. Normal full download: 200 OK, dynamic MIME type detected as audio/ogg; codecs=opus
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/test_audio_bundle/payload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("content-type").unwrap().to_str().unwrap(),
        "audio/ogg; codecs=opus"
    );
    assert_eq!(
        res.headers()
            .get("accept-ranges")
            .unwrap()
            .to_str()
            .unwrap(),
        "bytes"
    );
    assert_eq!(
        res.headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap(),
        "inline; filename=\"test_audio.ogg\""
    );

    // 2. Standard range request: bytes=0-9
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/test_audio_bundle/payload")
                .header("range", "bytes=0-9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap(),
        "bytes 0-9/50"
    );
    let body_bytes = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    assert_eq!(body_bytes.len(), 10);
    assert_eq!(&body_bytes[0..4], b"OggS");

    // 3. Suffix range request (used by Firefox to read end of stream/duration): bytes=-10
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/test_audio_bundle/payload")
                .header("range", "bytes=-10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap(),
        "bytes 40-49/50"
    );
    let body_bytes = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    assert_eq!(body_bytes.len(), 10);
    assert_eq!(body_bytes[0], 40);
    assert_eq!(body_bytes[9], 49);

    // 4. Suffix range request larger than file: bytes=-100 -> returns full file
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/test_audio_bundle/payload")
                .header("range", "bytes=-100")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap(),
        "bytes 0-49/50"
    );

    // 5. Out of bounds range request: bytes=100- -> 416 Range Not Satisfiable
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/test_audio_bundle/payload")
                .header("range", "bytes=100-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        res.headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap(),
        "bytes */50"
    );

    // 6. Video full download: 200 OK, dynamic MIME type detected as video/mp4, inline disposition
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/test_video_bundle/payload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("content-type").unwrap().to_str().unwrap(),
        "video/mp4"
    );
    assert_eq!(
        res.headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap(),
        "inline; filename=\"test_video.mp4\""
    );

    // 7. Video partial range request: bytes=4-7 -> 206 Partial Content (ftyp box)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/files/test_video_bundle/payload")
                .header("range", "bytes=4-7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap(),
        "bytes 4-7/64"
    );
    let body_bytes = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    assert_eq!(&body_bytes[..], b"ftyp");
}

fn url_encode(input: &str) -> String {
    let mut encoded = String::new();
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(&mut encoded, "%{:02X}", byte);
            }
        }
    }
    encoded
}
