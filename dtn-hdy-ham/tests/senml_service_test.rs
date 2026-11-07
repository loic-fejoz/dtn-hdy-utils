use axum::body::Body;
use axum::http::{Request, StatusCode};
use dtn_hdy_ham::config::{DashboardMetricConfig, HamConfig, SenmlConfig};
use dtn_hdy_ham::server::HamServer;
use dtn_hdy_ham::web::{AppState, create_router};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::Eid;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn test_senml_service_and_api_endpoints() {
    let config = HamConfig {
        callsign: Some("F5TEST".to_string()),
        node_id: "dtn://f5test/".to_string(),
        storage: dtn_hdy_ham::config::StorageConfig {
            in_memory: true,
            ..Default::default()
        },
        senml: Some(SenmlConfig {
            metrics: vec![
                DashboardMetricConfig {
                    name: "urn:dtn:sensor:bus_v".to_string(),
                    label: Some("Tension Bus".to_string()),
                },
                DashboardMetricConfig {
                    name: "urn:dtn:sensor:temp_cpu".to_string(),
                    label: Some("Température CPU".to_string()),
                },
            ],
        }),
        web: Some(dtn_hdy_ham::config::WebConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            static_dir: std::path::PathBuf::from("web/dist"),
        }),
        ..Default::default()
    };

    let server = HamServer::new(config).await.unwrap();
    server.bpa().start(true).await;

    // Register SenmlService with BPA
    let senml_service = Arc::new(dtn_hdy_ham::senml::SenmlService::new(
        server.senml_manager(),
        server.service_tracker(),
        server.keystore(),
    ));
    server
        .service_tracker()
        .register_service_with_type(
            hardy_bpv7::eid::Service::Dtn("senml".into()),
            senml_service,
            "Built-in SenML",
        )
        .await
        .expect("senml service registers");

    let injector = server.web_injector().expect("injector configured");
    let state = AppState {
        config: Arc::new(server.config().clone()),
        injector: injector.clone(),
        service_tracker: server.service_tracker(),
        cla_manager: server.cla_manager(),
        chat_manager: server.chat_manager(),
        internal_storage: server.internal_storage(),
        route_manager: server.route_manager(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
        keystore: server.keystore(),
        senml_manager: server.senml_manager(),
    };
    let app = create_router(state);

    // 1. GET /api/v1/senml/metrics - verify initial 2 metrics from YAML
    let req = Request::builder()
        .uri("/api/v1/senml/metrics")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(metrics.as_array().unwrap().len(), 2);
    assert_eq!(metrics[0]["name"], "urn:dtn:sensor:bus_v");
    assert_eq!(metrics[0]["custom_label"], "Tension Bus");
    assert_eq!(metrics[0]["is_visible"], true);

    // 2. Inject a SenML JSON bundle destined to dtn://f5test/senml
    let senml_payload = r#"[
        {"bn": "urn:dtn:sensor:", "bt": 1731000100, "bu": "V", "n": "bus_v", "v": 28.1},
        {"n": "temp_cpu", "u": "Cel", "v": 45.2},
        {"n": "new_unconfigured_metric", "u": "mA", "v": 320.0}
    ]"#;

    let source_eid: Eid = "dtn://remote_station/telemetry".parse().unwrap();
    let dest_eid: Eid = "dtn://f5test/senml".parse().unwrap();
    let (_bundle, binbundle) = Builder::new(source_eid, dest_eid)
        .with_payload(senml_payload.as_bytes().into())
        .with_lifetime(Duration::from_secs(3600))
        .build(CreationTimestamp::now())
        .unwrap();

    injector
        .inject(bytes::Bytes::from(binbundle.into_vec()))
        .await
        .unwrap();

    // Allow async dispatch into SenmlService
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 3. GET /api/v1/senml/metrics/latest?dashboard_only=true - should only return the 2 configured cards
    let req = Request::builder()
        .uri("/api/v1/senml/metrics/latest?dashboard_only=true")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let latest_dash: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(latest_dash.as_array().unwrap().len(), 2);
    assert_eq!(latest_dash[0]["name"], "urn:dtn:sensor:bus_v");
    assert_eq!(latest_dash[0]["value"], 28.1);
    assert_eq!(latest_dash[0]["unit"], "V");
    assert_eq!(latest_dash[0]["utc"], 1731000100000i64);

    // 4. GET /api/v1/senml/metrics/latest?id=urn:dtn:sensor:temp_cpu - filter test
    let req = Request::builder()
        .uri("/api/v1/senml/metrics/latest?id=urn:dtn:sensor:temp_cpu")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let filtered: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(filtered[0]["name"], "urn:dtn:sensor:temp_cpu");
    assert_eq!(filtered[0]["value"], 45.2);

    // 5. GET /api/v1/senml/history?id=urn:dtn:sensor:bus_v&start=0&end=2000000000000 - Open MCT history test
    let req = Request::builder()
        .uri("/api/v1/senml/history?id=urn:dtn:sensor:bus_v&start=0&end=2000000000000")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let history: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["utc"], 1731000100000i64);
    assert_eq!(history[0]["value"], 28.1);

    // 6. POST /api/v1/senml/dashboard/cards - Add the discovered metric to dashboard
    let add_payload = serde_json::json!({
        "name": "urn:dtn:sensor:new_unconfigured_metric",
        "label": "Consommation Courant"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/senml/dashboard/cards")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&add_payload).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // Verify dashboard now has 3 cards
    let req = Request::builder()
        .uri("/api/v1/senml/metrics/latest?dashboard_only=true")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let latest_dash3: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(latest_dash3.as_array().unwrap().len(), 3);
}
