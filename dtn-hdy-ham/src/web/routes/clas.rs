//! CLA runtime interaction and manual peer connection endpoint.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::web::AppState;

#[derive(Debug, Deserialize, Default)]
pub struct ConnectPeerRequest {
    pub peer: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConnectPeerQuery {
    pub peer: Option<String>,
}

/// Handler for `POST /api/v1/clas/connect` - Manually initiates connection to a CLA peer (e.g. TCPCLv4).
pub async fn connect_peer_handler(
    State(state): State<AppState>,
    Query(query): Query<ConnectPeerQuery>,
    body: Option<Json<ConnectPeerRequest>>,
) -> Response {
    let peer = query
        .peer
        .or_else(|| body.and_then(|b| b.0.peer))
        .unwrap_or_default();

    if peer.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Missing 'peer' parameter in body or query string" })),
        )
            .into_response();
    }

    match state.cla_manager.connect_peer(&peer).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "status": "connected",
                "peer": peer,
                "message": format!("Successfully connected to peer {peer}")
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string(), "peer": peer })),
        )
            .into_response(),
    }
}
