//! Config endpoint handler.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::web::AppState;

/// Handles `GET /api/v1/config`.
pub async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    let config = &state.config;

    let response = json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "callsign": config.callsign,
        "node_id": config.node_id,
        "admin_endpoints": config.admin_endpoints,
        "storage": {
            "in_memory": config.storage.in_memory,
            "sqlite_path": config.storage.sqlite_path,
        },
        "grpc": {
            "enabled": config.grpc.enabled,
            "address": config.grpc.address.to_string(),
        },
        "web": config.web.as_ref().map(|w| json!({
            "address": w.address.to_string(),
            "static_dir": w.static_dir,
        })),
        "clas": config.clas,
        "routes": config.routes,
    });

    (StatusCode::OK, Json(response))
}
