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
        schedule: None,
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
        schedule: None,
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
        schedule: None,
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
rfc9171-validity:
  primary-block-integrity: false
  bundle-age-required: true
"#;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test_config.yaml");
    std::fs::write(&file_path, yaml).unwrap();

    let config = HamConfig::load(Some(&file_path)).expect("YAML config should parse");
    assert_eq!(config.node_id, "dtn://f4jxq.ampr.org/");
    assert_eq!(config.admin_endpoints, vec!["ipn:1.0"]);
    assert_eq!(config.clas.len(), 3);
    assert_eq!(config.routes.len(), 1);
    let rfc = config.rfc9171_validity.expect("rfc9171 should parse");
    assert_eq!(rfc.primary_block_integrity, Some(false));
    assert_eq!(rfc.bundle_age_required, Some(true));
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
    let cla_manager = server.cla_manager();
    let chat_manager = server.chat_manager();
    let internal_storage = server.internal_storage();
    let state = AppState {
        config: Arc::new(config),
        injector,
        service_tracker: service_tracker.clone(),
        cla_manager,
        chat_manager,
        internal_storage,
        route_manager: server.route_manager(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
        keystore: server.keystore(),
        senml_manager: server.senml_manager(),
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

#[tokio::test]
async fn test_wizard_tcpcl_recurring_schedule_startup_never_panics() {
    let dir = tempfile::tempdir().unwrap();
    let config_file = dir.path().join("stata-wizard.yaml");

    let yaml = r#"
# dtn-hdy-ham configuration generated by wizard
callsign: "STATA"
node-id: "dtn://stata/"

storage:
  in-memory: true

grpc:
  enabled: false

web:
  address: "127.0.0.1:0"

rfc9171-validity:
  primary-block-integrity: false
  bundle-age-required: false

clas:
  - name: "tcpcl-f4jxq"
    type: "tcpclv4"
    address: "127.0.0.1:0"
    require-tls: false
    peers:
      - "127.0.0.1:4556"
    schedule:
      type: "recurring"
      cron: "*/43 * * * *"
      duration: "300s"

routes:
  - pattern: "dtn://f4jxq-9/*"
    cla: "tcpcl-f4jxq"
    next-hop: "dtn://f4jxq/"
  - pattern: "dtn://**"
    cla: "tcpcl-f4jxq"
    next-hop: "dtn://f4jxq/"
"#;

    std::fs::write(&config_file, yaml).unwrap();

    let config = HamConfig::load(Some(&config_file)).expect("valid YAML config");
    assert_eq!(config.callsign, Some("STATA".to_string()));
    assert_eq!(config.node_id, "dtn://stata/");
    assert_eq!(config.clas.len(), 1);
    assert_eq!(config.routes.len(), 2);

    // Initializing server must never panic even with TCPCL contact coordinator schedule
    let server = HamServer::new(config)
        .await
        .expect("HamServer should build without any panic");

    server.bpa().start(false).await;
    server.shutdown().await;
}

#[tokio::test]
async fn test_stata_manual_toml_and_yaml_parity() {
    let yaml_path = std::path::Path::new("../examples/stata-manual.yaml");
    let toml_path = std::path::Path::new("../examples/stata-manual.toml");

    let yaml_config = HamConfig::load(Some(yaml_path)).expect("valid YAML config");
    let toml_config = HamConfig::load(Some(toml_path)).expect("valid TOML config");

    assert_eq!(yaml_config.callsign, toml_config.callsign);
    assert_eq!(yaml_config.node_id, toml_config.node_id);
    assert_eq!(yaml_config.storage.in_memory, toml_config.storage.in_memory);
    assert_eq!(yaml_config.storage.data_dir, toml_config.storage.data_dir);
    assert_eq!(yaml_config.grpc.enabled, toml_config.grpc.enabled);
    assert_eq!(yaml_config.grpc.address, toml_config.grpc.address);
    assert_eq!(
        yaml_config.web.as_ref().map(|w| w.address),
        toml_config.web.as_ref().map(|w| w.address)
    );
    assert_eq!(
        yaml_config
            .rfc9171_validity
            .as_ref()
            .and_then(|r| r.primary_block_integrity),
        toml_config
            .rfc9171_validity
            .as_ref()
            .and_then(|r| r.primary_block_integrity)
    );
    assert_eq!(
        yaml_config
            .rfc9171_validity
            .as_ref()
            .and_then(|r| r.bundle_age_required),
        toml_config
            .rfc9171_validity
            .as_ref()
            .and_then(|r| r.bundle_age_required)
    );
    assert_eq!(yaml_config.clas.len(), toml_config.clas.len());
    assert_eq!(yaml_config.clas[0].name, toml_config.clas[0].name);
    assert_eq!(yaml_config.routes.len(), toml_config.routes.len());
    assert_eq!(yaml_config.routes[0].pattern, toml_config.routes[0].pattern);
    assert_eq!(yaml_config.routes[1].pattern, toml_config.routes[1].pattern);
}

#[tokio::test]
async fn test_config_example_parity() {
    let toml_path = std::path::Path::new("config.example.toml");
    let yaml_path = std::path::Path::new("config.example.yaml");

    let toml_cfg = HamConfig::load(Some(toml_path)).expect("config.example.toml must be valid");
    let yaml_cfg = HamConfig::load(Some(yaml_path)).expect("config.example.yaml must be valid");

    assert_eq!(toml_cfg.callsign, yaml_cfg.callsign);
    assert_eq!(toml_cfg.node_id, yaml_cfg.node_id);
    assert_eq!(toml_cfg.storage.data_dir, yaml_cfg.storage.data_dir);
    assert_eq!(toml_cfg.storage.in_memory, yaml_cfg.storage.in_memory);
    assert_eq!(
        toml_cfg.application.quota.max_storage,
        yaml_cfg.application.quota.max_storage
    );
    assert_eq!(toml_cfg.grpc.enabled, yaml_cfg.grpc.enabled);
    assert_eq!(toml_cfg.grpc.address, yaml_cfg.grpc.address);
    assert_eq!(
        toml_cfg.web.as_ref().map(|w| w.address),
        yaml_cfg.web.as_ref().map(|w| w.address)
    );
    assert_eq!(toml_cfg.clas.len(), yaml_cfg.clas.len());
    assert_eq!(toml_cfg.clas[0].name, yaml_cfg.clas[0].name);
    assert_eq!(toml_cfg.routes.len(), yaml_cfg.routes.len());
    assert_eq!(toml_cfg.routes[0].pattern, yaml_cfg.routes[0].pattern);
}

#[tokio::test]
async fn test_empty_config_file_behavior() {
    let dir = tempfile::tempdir().unwrap();

    // 1. 0-byte YAML file
    let empty_yaml = dir.path().join("empty.yaml");
    std::fs::write(&empty_yaml, "").unwrap();

    assert!(HamConfig::is_empty_file(&empty_yaml));
    assert_eq!(HamConfig::find_config(Some(&empty_yaml)), None);
    let err_yaml = HamConfig::load(Some(&empty_yaml)).unwrap_err();
    assert!(err_yaml.to_string().contains("is empty"));

    // 2. Comments/whitespace-only TOML file
    let empty_toml = dir.path().join("empty.toml");
    std::fs::write(
        &empty_toml,
        "# Initial empty config\n   \n# More comments\n",
    )
    .unwrap();

    assert!(HamConfig::is_empty_file(&empty_toml));
    assert_eq!(HamConfig::find_config(Some(&empty_toml)), None);
    let err_toml = HamConfig::load(Some(&empty_toml)).unwrap_err();
    assert!(err_toml.to_string().contains("is empty"));

    // 3. Wizard started with empty.toml as suggested save path saves valid TOML and reloads cleanly
    let input = b"F4XYZ\n\n\n\n\n";
    let mut reader = &input[..];
    let mut output = Vec::new();

    let cfg = dtn_hdy_ham::run_wizard(&mut reader, &mut output, Some(empty_toml.clone())).unwrap();
    assert_eq!(cfg.callsign, Some("F4XYZ".to_string()));
    assert!(!HamConfig::is_empty_file(&empty_toml));
    assert_eq!(
        HamConfig::find_config(Some(&empty_toml)),
        Some(empty_toml.clone())
    );

    let reloaded = HamConfig::load(Some(&empty_toml)).expect("should load valid TOML");
    assert_eq!(reloaded.callsign, Some("F4XYZ".to_string()));
    assert_eq!(reloaded.node_id, "dtn://f4xyz/");
}
