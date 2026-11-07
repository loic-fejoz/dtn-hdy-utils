//! REST and SSE handlers for SenML telemetry, metrics dictionary and Open MCT integration.

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

use crate::web::AppState;

/// Query parameters for `GET /api/v1/senml/metrics/latest`.
#[derive(Debug, Deserialize, Default)]
pub struct LatestMetricsQuery {
    pub id: Option<String>,
    pub ids: Option<String>,
    pub dashboard_only: Option<bool>,
}

/// Query parameters for `GET /api/v1/senml/history` (Open MCT Telemetry API).
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub id: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
}

/// Request payload for adding a card to the SenML dashboard.
#[derive(Debug, Deserialize)]
pub struct AddCardRequest {
    pub name: String,
    #[serde(alias = "custom_label")]
    pub label: Option<String>,
}

/// Request payload for updating a card's alias or display order.
#[derive(Debug, Deserialize)]
pub struct UpdateCardRequest {
    #[serde(alias = "custom_label")]
    pub label: Option<String>,
    #[serde(alias = "display_order")]
    pub order: Option<i64>,
}

fn internal_error<E: std::fmt::Display>(err: E) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": err.to_string() })),
    )
}

/// GET /api/v1/senml/metrics - Returns the list of all known telemetry metrics.
pub async fn get_metrics(State(state): State<AppState>) -> impl IntoResponse {
    match state.senml_manager.storage().list_metrics() {
        Ok(metrics) => (StatusCode::OK, Json(json!(metrics))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/senml/metrics/latest - Returns latest point for metrics (with optional filtering).
pub async fn get_latest_metrics(
    State(state): State<AppState>,
    Query(query): Query<LatestMetricsQuery>,
) -> impl IntoResponse {
    let mut filters = Vec::new();
    if let Some(ref single) = query.id {
        filters.push(single.trim().to_string());
    }
    if let Some(ref multiple) = query.ids {
        for part in multiple.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                filters.push(trimmed.to_string());
            }
        }
    }

    let filter_slice = if filters.is_empty() {
        None
    } else {
        Some(filters.as_slice())
    };

    let dashboard_only = query.dashboard_only.unwrap_or(false);

    match state
        .senml_manager
        .storage()
        .get_latest_metrics(filter_slice, dashboard_only)
    {
        Ok(latest) => (StatusCode::OK, Json(json!(latest))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/senml/history - Historical time series for Open MCT (`?id=...&start=...&end=...`).
pub async fn get_history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let start = query.start.unwrap_or(0);
    let end = query.end.unwrap_or_else(|| {
        (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64 + 86_400_000
    });

    match state
        .senml_manager
        .storage()
        .get_history(&query.id, start, end)
    {
        Ok(history) => (StatusCode::OK, Json(json!(history))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/senml/stream - Real-time Server-Sent Events stream for telemetry (Open MCT compatible).
pub async fn get_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.senml_manager.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(point) => {
                let data = serde_json::to_string(&point).unwrap_or_default();
                Some(Ok(Event::default().event("telemetry").data(data)))
            }
            Err(_) => None,
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// GET /api/v1/senml/dashboard/cards - Returns all cards configured for the active dashboard.
pub async fn get_dashboard_cards(State(state): State<AppState>) -> impl IntoResponse {
    match state.senml_manager.storage().get_latest_metrics(None, true) {
        Ok(cards) => (StatusCode::OK, Json(json!(cards))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// POST /api/v1/senml/dashboard/cards - Adds a metric card to the active dashboard.
pub async fn add_dashboard_card(
    State(state): State<AppState>,
    Json(payload): Json<AddCardRequest>,
) -> impl IntoResponse {
    let name = payload.name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Metric name cannot be empty" })),
        )
            .into_response();
    }

    match state
        .senml_manager
        .storage()
        .add_dashboard_card(name, payload.label)
    {
        Ok(entry) => (StatusCode::CREATED, Json(json!(entry))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// PUT /api/v1/senml/dashboard/cards/{id} - Updates label or order of a dashboard card.
pub async fn update_dashboard_card(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateCardRequest>,
) -> impl IntoResponse {
    match state
        .senml_manager
        .storage()
        .update_dashboard_card(id, payload.label, payload.order)
    {
        Ok(entry) => (StatusCode::OK, Json(json!(entry))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// DELETE /api/v1/senml/dashboard/cards/{id} - Removes a card from the dashboard.
pub async fn remove_dashboard_card(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i64>,
) -> impl IntoResponse {
    match state.senml_manager.storage().remove_dashboard_card(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "status": "removed", "id": id })),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// DELETE /api/v1/senml/dashboard/cards - Clears all cards from the dashboard.
pub async fn clear_dashboard_cards(State(state): State<AppState>) -> impl IntoResponse {
    match state.senml_manager.storage().clear_dashboard() {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "cleared" }))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}
