use dtn_hdy_ham::config::cla::{ClaConfig, ClaType, Tcpclv4HamConfig};
use dtn_hdy_ham::{HamConfig, HamServer};
use hardy_hqfbp_cla::Config as HqfbpConfig;
use hardy_ltp_cla::Config as LtpConfig;
use std::time::Duration;

#[tokio::test]
async fn test_ham_server_in_memory_startup() {
    let mut config = HamConfig {
        node_id: "dtn://f4jxq.ampr.org/".to_string(),
        storage: dtn_hdy_ham::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: dtn_hdy_ham::config::GrpcConfig {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };

    // 1. Add TCPCLv4 CLA with amateur radio compliance (require_tls: false)
    config.clas.push(ClaConfig {
        name: "ham-tcpcl".to_string(),
        cla_type: ClaType::TcpClv4(Tcpclv4HamConfig {
            address: Some("127.0.0.1:0".parse().unwrap()),
            require_tls: Some(false),
            ..Default::default()
        }),
        policy: None,
    });

    // 2. Add LTP CLA with UDP transport
    let ltp_cfg = LtpConfig {
        engine_id: hardy_ltp::types::EngineId(42),
        transport: hardy_ltp_cla::TransportMode::Udp {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
        },
        ..Default::default()
    };
    config.clas.push(ClaConfig {
        name: "ham-ltp".to_string(),
        cla_type: ClaType::Ltp(ltp_cfg),
        policy: None,
    });

    // 3. Add HQFBP CLA with UDP transport
    let hqfbp_cfg = HqfbpConfig {
        callsign: Some("F4JXQ-1".to_string()),
        transport: hardy_hqfbp_cla::TransportMode::Udp {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            peer_addr: None,
        },
        ..Default::default()
    };
    config.clas.push(ClaConfig {
        name: "ham-hqfbp".to_string(),
        cla_type: ClaType::Hqfbp(hqfbp_cfg),
        policy: None,
    });

    let server = HamServer::new(config)
        .await
        .expect("HamServer should build");
    let bpa = server.bpa();

    // Verify BPA starts and stops cleanly
    bpa.start(false).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    server.shutdown().await;
}

#[test]
fn test_ham_config_yaml_parsing() {
    let yaml = r#"
node-id: "dtn://f4jxq.ampr.org/"
admin-endpoints:
  - "ipn:1.0"
storage:
  in-memory: true
grpc:
  enabled: true
  address: "127.0.0.1:50051"
clas:
  - name: tcpcl-44net-gateway
    type: tcpclv4
    address: "127.0.0.1:4556"
    require-tls: false
    peers:
      - "44.27.131.233:4556"
  - name: kiss-tnc-vhf
    type: hqfbp
    callsign: "F4JXQ-1"
    transport:
      type: tcp
      address: "127.0.0.1:8001"
  - name: ltp-udp-cislunar
    type: ltp
    engine-id: 1001
    transport:
      type: udp
      bind-addr: "0.0.0.0:1113"
routes:
  - pattern: "dtn://*/*"
    cla: "tcpcl-44net-gateway"
"#;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test_config.yaml");
    std::fs::write(&file_path, yaml).unwrap();

    let config = HamConfig::load(Some(&file_path)).expect("YAML config should parse");
    assert_eq!(config.node_id, "dtn://f4jxq.ampr.org/");
    assert_eq!(config.admin_endpoints, vec!["ipn:1.0"]);
    assert_eq!(config.clas.len(), 3);
    assert_eq!(config.routes.len(), 1);
}

#[test]
fn test_radiant_callsign_and_mercury_yaml() {
    let yaml = r#"
callsign: "STATA"
storage:
  in-memory: true
clas:
  - name: mercury-hf
    type: mercury-arq
    control-addr: "127.0.0.1:8300"
    data-addr: "127.0.0.1:8301"
    peers:
      - callsign: "STATB"
        schedule:
          type: recurring
          cron: "0 */15 * * * *"
          duration: "3m"
"#;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("radiant_mercury.yaml");
    std::fs::write(&file_path, yaml).unwrap();

    let config = HamConfig::load(Some(&file_path)).expect("YAML config should parse");
    // RADIANT callsign default for node-id
    assert_eq!(config.callsign, Some("STATA".to_string()));
    assert_eq!(config.node_id, "dtn://stata/");

    assert_eq!(config.clas.len(), 1);
    match &config.clas[0].cla_type {
        ClaType::MercuryArq(m) => {
            // Inherited callsign
            assert_eq!(m.my_callsign, Some("STATA".to_string()));
            assert_eq!(m.peers.len(), 1);
            assert_eq!(m.peers[0].callsign, "STATB");
            // RADIANT peer node-id defaulting
            assert_eq!(m.peers[0].effective_node_id(), "dtn://statb/");
            assert!(matches!(
                m.peers[0].schedule,
                Some(dtn_hdy_ham::config::cla::ScheduleConfig::Recurring { .. })
            ));
        }
        _ => panic!("expected MercuryArq CLA"),
    }
}

#[tokio::test]
async fn test_ham_web_api_routes() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dtn_hdy_ham::web::{AppState, create_router};
    use std::sync::Arc;
    use tower::ServiceExt;

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
    server.bpa().start(false).await;

    // Use router with AppState connected to the real server injector
    let injector = server.web_injector().expect("injector configured");
    let service_tracker = server.service_tracker();
    let state = AppState {
        config: Arc::new(config),
        injector,
        service_tracker: service_tracker.clone(),
    };
    let app = create_router(state);

    // 1. Test GET /api/v1/config
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(v["callsign"], "STATA");
    assert_eq!(v["node_id"], "dtn://stata/");
    assert_eq!(v["status"], "ok");

    // 2. Test POST /api/v1/bundles with valid payload
    let send_req = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/bundles")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"destination": "dtn://statb/incoming", "payload": "hello world"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(send_req.status(), StatusCode::CREATED);
    let send_bytes = axum::body::to_bytes(send_req.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let send_json: serde_json::Value = serde_json::from_slice(&send_bytes).unwrap();
    assert_eq!(send_json["status"], "sent");
    assert!(send_json["bundle_id"].as_str().is_some());
    assert_eq!(send_json["destination"], "dtn://statb/incoming");

    // 3. Test POST /api/v1/bundles with invalid destination
    let bad_req = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/bundles")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"destination": "", "payload": "hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(bad_req.status(), StatusCode::BAD_REQUEST);

    // 4. Test GET /api/v1/services before and after registration
    let svc_req = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/services")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(svc_req.status(), StatusCode::OK);
    let svc_bytes = axum::body::to_bytes(svc_req.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let svcs: Vec<serde_json::Value> = serde_json::from_slice(&svc_bytes).unwrap();
    assert_eq!(svcs.len(), 0);

    // Register a service via service_tracker
    let echo = std::sync::Arc::new(hardy_echo_service::EchoService::new());
    let service_id = hardy_bpv7::eid::Service::Dtn("echo".into());
    let reg_eid = service_tracker
        .register_service_with_type(service_id, echo, "Built-in Echo")
        .await
        .expect("echo should register");

    assert_eq!(reg_eid.to_string(), "dtn://stata/echo");

    let svc_req2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/services")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(svc_req2.status(), StatusCode::OK);
    let svc_bytes2 = axum::body::to_bytes(svc_req2.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let svcs2: Vec<serde_json::Value> = serde_json::from_slice(&svc_bytes2).unwrap();
    assert_eq!(svcs2.len(), 1);
    assert_eq!(svcs2[0]["eid"], "dtn://stata/echo");
    assert_eq!(svcs2[0]["type"], "Built-in Echo");
    assert!(svcs2[0]["registered_at"].as_str().is_some());

    server.shutdown().await;
}
