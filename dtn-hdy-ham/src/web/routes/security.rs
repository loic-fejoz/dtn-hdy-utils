use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;

use crate::security::{Direction, VerifyPolicy, parse_key_input};
use crate::web::AppState;

#[derive(Debug, Deserialize)]
pub struct UpdatePolicyRequest {
    pub policy: VerifyPolicy,
}

#[derive(Debug, Deserialize)]
pub struct AddKeyRequest {
    pub direction: Direction,
    pub pattern: String,
    pub secret: String,
    pub label: Option<String>,
}

/// GET /api/v1/security/policy
pub async fn get_policy(State(state): State<AppState>) -> impl IntoResponse {
    let policy = state.keystore.get_policy();
    Json(json!({ "policy": policy }))
}

/// PUT /api/v1/security/policy
pub async fn update_policy(
    State(state): State<AppState>,
    Json(payload): Json<UpdatePolicyRequest>,
) -> impl IntoResponse {
    match state.keystore.set_policy(payload.policy) {
        Ok(()) => {
            tracing::info!("Updated BPSec verification policy to {:?}", payload.policy);
            (
                StatusCode::OK,
                Json(json!({
                    "status": "updated",
                    "policy": payload.policy
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to update policy: {e}") })),
        )
            .into_response(),
    }
}

/// GET /api/v1/security/keys
pub async fn list_keys(State(state): State<AppState>) -> impl IntoResponse {
    let keys = state.keystore.list_keys();
    Json(keys)
}

/// POST /api/v1/security/keys
pub async fn add_key(
    State(state): State<AppState>,
    Json(payload): Json<AddKeyRequest>,
) -> impl IntoResponse {
    let pattern = payload.pattern.trim();
    if pattern.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Pattern cannot be empty" })),
        )
            .into_response();
    }

    let secret_bytes = parse_key_input(&payload.secret);
    if secret_bytes.len() < 16 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Secret key must be at least 16 bytes (or 32 hex characters)" })),
        )
            .into_response();
    }

    match state.keystore.add_key(
        payload.direction,
        pattern.to_string(),
        secret_bytes,
        payload.label,
    ) {
        Ok(info) => {
            tracing::info!(
                "Added BPSec key {} for pattern '{}' (direction: {:?})",
                info.id,
                info.pattern,
                info.direction
            );
            (StatusCode::CREATED, Json(info)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to add key: {e}") })),
        )
            .into_response(),
    }
}

/// DELETE /api/v1/security/keys/{id}
pub async fn delete_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.keystore.remove_key(&id) {
        Ok(true) => {
            tracing::info!("Removed BPSec key {}", id);
            (
                StatusCode::OK,
                Json(json!({ "status": "deleted", "id": id })),
            )
                .into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Key not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to delete key: {e}") })),
        )
            .into_response(),
    }
}
