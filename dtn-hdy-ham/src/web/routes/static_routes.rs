//! Web API handlers for dynamic static routing.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::config::RouteConfig;
use crate::web::AppState;

/// Query parameters for deleting a static route.
#[derive(Debug, Deserialize)]
pub struct DeleteRouteQuery {
    pub pattern: String,
}

/// GET /api/v1/routes - Lists all active static routes.
pub async fn list_routes(State(state): State<AppState>) -> impl IntoResponse {
    let routes = state.route_manager.list_routes().await;
    Json(routes)
}

/// POST /api/v1/routes - Adds a new static route and injects it immediately into the BPA RIB.
pub async fn add_route(
    State(state): State<AppState>,
    Json(payload): Json<RouteConfig>,
) -> Response {
    if payload.pattern.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Route pattern cannot be empty" })),
        )
            .into_response();
    }

    match state.route_manager.add_route(payload.clone()).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({
                "status": "created",
                "route": payload
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Failed to add static route: {e}") })),
        )
            .into_response(),
    }
}

/// DELETE /api/v1/routes?pattern=... - Removes a static route from the BPA RIB and config.
pub async fn delete_route(
    State(state): State<AppState>,
    Query(params): Query<DeleteRouteQuery>,
) -> Response {
    let pattern = params.pattern.trim();
    if pattern.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Pattern parameter cannot be empty" })),
        )
            .into_response();
    }

    match state.route_manager.delete_route(pattern).await {
        Ok(true) => (
            StatusCode::OK,
            Json(json!({
                "status": "deleted",
                "pattern": pattern
            })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Route pattern '{pattern}' not found") })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to delete static route: {e}") })),
        )
            .into_response(),
    }
}
