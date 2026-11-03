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
