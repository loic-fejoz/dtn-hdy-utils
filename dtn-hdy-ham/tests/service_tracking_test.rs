use std::sync::Arc;
use std::time::Duration;

use dtn_hdy_ham::config::HamConfig;
use dtn_hdy_ham::server::HamServer;
use hardy_async::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service, ServiceSink};
use hardy_bpa::stream::{Receiver, Segment};
use hardy_bpv7::bundle::Id;
use hardy_bpv7::eid::{Eid, Service as Bpv7Service};
use hardy_bpv7::status_report::ReasonCode;
use hardy_proto::client::RemoteBpa;
use time::OffsetDateTime;
use tokio::sync::OnceCell;

struct MockRemoteService {
    sink: OnceCell<Box<dyn ServiceSink>>,
}

#[async_trait]
impl Service for MockRemoteService {
    async fn on_register(&self, _source: &Eid, sink: Box<dyn ServiceSink>) {
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {}

    async fn on_deliver(
        &self,
        _bundle_id: &Id,
        _expiry: OffsetDateTime,
        _total_len: u64,
        _stream: &mut dyn Receiver<Segment>,
    ) -> hardy_bpa::services::Result<()> {
        Ok(())
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &Id,
        _from: &Eid,
        _kind: hardy_bpa::services::StatusNotify,
        _reason: ReasonCode,
        _timestamp: Option<OffsetDateTime>,
    ) {
    }
}

#[tokio::test]
async fn test_active_services_tracking_and_grpc_lifecycle() {
    let config = HamConfig {
        callsign: Some("F4TEST".to_string()),
        node_id: "dtn://f4test/".to_string(),
        storage: dtn_hdy_ham::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        grpc: dtn_hdy_ham::config::GrpcConfig {
            enabled: true,
            address: "127.0.0.1:54321".parse().unwrap(),
        },
        web: Some(dtn_hdy_ham::config::WebConfig {
            address: "127.0.0.1:8888".parse().unwrap(),
            static_dir: std::path::PathBuf::from("./web/dist"),
        }),
        ..Default::default()
    };

    let server = HamServer::new(config.clone())
        .await
        .expect("HamServer builds");

    // Spawn the server in background
    let s_handle = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // Give server a moment to start gRPC and register permanent Echo service
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Check GET /api/v1/services via HTTP client (or local reqwest)
    let client = reqwest::Client::new();
    let res = client
        .get("http://127.0.0.1:8888/api/v1/services")
        .send()
        .await
        .expect("services request should succeed");

    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let svcs: Vec<serde_json::Value> = res.json().await.expect("valid JSON response");

    // Permanent Echo service should always be active
    assert_eq!(svcs.len(), 1);
    assert_eq!(svcs[0]["eid"], "dtn://f4test/echo");
    assert_eq!(svcs[0]["type"], "Built-in Echo");
    assert!(svcs[0]["registered_at"].as_str().is_some());

    // Connect remote service via gRPC (simulating dtnprint)
    let remote_bpa = RemoteBpa::new("http://127.0.0.1:54321".to_string());
    let mock_svc = Arc::new(MockRemoteService {
        sink: OnceCell::new(),
    });

    let service_id = Bpv7Service::Dtn("incoming".into());
    let reg_eid = remote_bpa
        .register_service(service_id, mock_svc.clone())
        .await
        .expect("remote gRPC service should register");

    assert_eq!(reg_eid.to_string(), "dtn://f4test/incoming");

    // Wait a brief moment for registration event to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify both echo and incoming are listed in /api/v1/services
    let res2 = client
        .get("http://127.0.0.1:8888/api/v1/services")
        .send()
        .await
        .expect("services request should succeed");
    let svcs2: Vec<serde_json::Value> = res2.json().await.expect("valid JSON response");
    assert_eq!(svcs2.len(), 2);

    let eids: Vec<&str> = svcs2.iter().filter_map(|s| s["eid"].as_str()).collect();
    assert!(eids.contains(&"dtn://f4test/echo"));
    assert!(eids.contains(&"dtn://f4test/incoming"));

    let incoming_svc = svcs2
        .iter()
        .find(|s| s["eid"] == "dtn://f4test/incoming")
        .unwrap();
    assert_eq!(incoming_svc["type"], "gRPC Client");

    // Now unregister remote service (simulating dtnprint stopping)
    if let Some(sink) = mock_svc.sink.get() {
        sink.unregister().await;
    }

    // Wait a brief moment for unregistration to propagate
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify incoming service is gone and only echo remains
    let res3 = client
        .get("http://127.0.0.1:8888/api/v1/services")
        .send()
        .await
        .expect("services request should succeed");
    let svcs3: Vec<serde_json::Value> = res3.json().await.expect("valid JSON response");
    assert_eq!(svcs3.len(), 1);
    assert_eq!(svcs3[0]["eid"], "dtn://f4test/echo");

    s_handle.abort();
}
