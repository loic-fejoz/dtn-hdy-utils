use axum::body::Body;
use axum::http::{Request, StatusCode};
use dtn_hdy_ham::config::RouteConfig;
use dtn_hdy_ham::web::{AppState, create_router};
use dtn_hdy_ham::{HamConfig, HamServer};
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn test_routes_api_crud_and_live_reload() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("station_config.yaml");

    let initial_yaml = r#"
callsign: "F4TEST"
node-id: "dtn://f4test/"
storage:
  in-memory: true
grpc:
  enabled: false
web:
  address: "127.0.0.1:0"
routes:
  - pattern: "dtn://f4jxq-9/*"
    cla: "tcpcl-default"
    next-hop: "dtn://f4jxq/"
"#;
    std::fs::write(&config_path, initial_yaml).unwrap();

    let mut config = HamConfig::load(Some(&config_path)).expect("config loads");
    config.storage.in_memory = true;
    config.grpc.enabled = false;

    let server = HamServer::new(config.clone()).await.expect("server starts");
    server.bpa().start(false).await;

    let injector = server.web_injector().expect("injector configured");
    let service_tracker = server.service_tracker();
    let cla_manager = server.cla_manager();
    let chat_manager = server.chat_manager();
    let internal_storage = server.internal_storage();
    let route_manager = server.route_manager();

    let state = AppState {
        config: Arc::new(config),
        injector,
        service_tracker,
        cla_manager: cla_manager.clone(),
        chat_manager,
        internal_storage,
        route_manager: route_manager.clone(),
        log_buffer: dtn_hdy_ham::LogBuffer::global(),
    };
    let app = create_router(state);

    // 1. GET /api/v1/routes - initial route from config
    let req = Request::builder()
        .uri("/api/v1/routes")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let routes: Vec<RouteConfig> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].pattern, "dtn://f4jxq-9/*");
    assert_eq!(routes[0].next_hop.as_deref(), Some("dtn://f4jxq/"));

    // 2. POST /api/v1/routes - add a new default gateway route "dtn://**"
    let new_route = json!({
        "pattern": "dtn://**",
        "cla": "tcpcl-default",
        "next-hop": "dtn://f4jxq/"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/routes")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&new_route).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // Verify GET /api/v1/routes returns 2 routes now
    let req = Request::builder()
        .uri("/api/v1/routes")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let routes: Vec<RouteConfig> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[1].pattern, "dtn://**");

    // Verify YAML file was updated on disk
    let updated_yaml = std::fs::read_to_string(&config_path).unwrap();
    assert!(updated_yaml.contains("dtn://**"));
    assert!(updated_yaml.contains("dtn://f4jxq-9/*"));

    // 3. DELETE /api/v1/routes?pattern=dtn://f4jxq-9/*
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/routes?pattern=dtn://f4jxq-9/*")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Verify GET /api/v1/routes now has only 1 route
    let req = Request::builder()
        .uri("/api/v1/routes")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let routes: Vec<RouteConfig> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].pattern, "dtn://**");

    // 4. Test GET /api/v1/clas/status
    let addr: std::net::SocketAddr = "44.27.131.233:4556".parse().unwrap();
    cla_manager
        .set_peer_status(
            addr,
            dtn_hdy_ham::cla_manager::PeerConnectionStatus::Connected,
        )
        .await;

    let req = Request::builder()
        .uri("/api/v1/clas/status")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let statuses: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0]["address"], "44.27.131.233:4556");
    assert_eq!(statuses[0]["status"], "connected");

    server.shutdown().await;
}
