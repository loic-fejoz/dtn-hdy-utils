//! Active DTN services, logging and SSE event handlers.

use std::convert::Infallible;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;

use crate::log_buffer::LogEntry;
use crate::service_tracker::{ServiceEvent, ServiceInfo};
use crate::web::AppState;

/// GET /api/v1/services - Returns list of active DTN services.
pub async fn get_services(State(state): State<AppState>) -> Json<Vec<ServiceInfo>> {
    Json(state.service_tracker.list_services())
}

/// Response payload for `GET /api/v1/logs`.
#[derive(Debug, Serialize, Deserialize)]
pub struct LogsResponse {
    pub logs: Vec<LogEntry>,
    pub count: usize,
    pub capacity: usize,
}

/// GET /api/v1/logs - Returns the snapshot of buffered log records.
pub async fn get_logs(State(state): State<AppState>) -> Json<LogsResponse> {
    let logs = state.log_buffer.list_entries();
    let count = logs.len();
    let capacity = state.log_buffer.capacity();
    Json(LogsResponse {
        logs,
        count,
        capacity,
    })
}

/// Query parameters for `GET /api/v1/events`.
#[derive(Debug, Deserialize, Default)]
pub struct SseParams {
    pub logs: Option<bool>,
}

/// GET /api/v1/events - Server-Sent Events stream for real-time station and service updates.
pub async fn sse_events(
    State(state): State<AppState>,
    Query(params): Query<SseParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Send initial snapshot immediately upon connection
    let initial_services = state.service_tracker.list_services();
    let initial_json = serde_json::to_string(&initial_services).unwrap_or_default();
    let initial_event = Event::default().event("services_update").data(initial_json);

    // Stream subsequent updates from the service tracker broadcast channel
    let rx = state.service_tracker.subscribe();
    let business_stream = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(ServiceEvent::ServicesUpdate { services }) => {
                let data = serde_json::to_string(&services).unwrap_or_default();
                Some(Ok(Event::default().event("services_update").data(data)))
            }
            Ok(ServiceEvent::BundlesUpdate) => {
                Some(Ok(Event::default().event("bundles_update").data("{}")))
            }
            Ok(ServiceEvent::ChatMessage { message }) => {
                let data = serde_json::to_string(&message).unwrap_or_default();
                Some(Ok(Event::default().event("chat_message").data(data)))
            }
            Ok(ServiceEvent::FileReceived { file }) => {
                let data = serde_json::to_string(&file).unwrap_or_default();
                Some(Ok(Event::default().event("file_received").data(data)))
            }
            Ok(ServiceEvent::ClaStateChanged {
                name,
                cla_type,
                state,
                details,
            }) => {
                let payload = serde_json::json!({
                    "name": name,
                    "cla_type": cla_type,
                    "state": state,
                    "details": details,
                });
                let data = serde_json::to_string(&payload).unwrap_or_default();
                Some(Ok(Event::default().event("cla_state_changed").data(data)))
            }
            Ok(ServiceEvent::ContactWindow { name, action, cla }) => {
                let payload = serde_json::json!({
                    "name": name,
                    "action": action,
                    "cla": cla,
                });
                let data = serde_json::to_string(&payload).unwrap_or_default();
                Some(Ok(Event::default().event("contact_window").data(data)))
            }
            Err(_) => None,
        }
    });

    let stream: futures_util::stream::BoxStream<'static, Result<Event, Infallible>> =
        if params.logs.unwrap_or(false) {
            let log_rx = state.log_buffer.subscribe();
            let log_stream = BroadcastStream::new(log_rx).filter_map(|res| async move {
                match res {
                    Ok(entry) => {
                        let data = serde_json::to_string(&entry).unwrap_or_default();
                        Some(Ok(Event::default().event("log_entry").data(data)))
                    }
                    Err(_) => None,
                }
            });
            futures_util::stream::select(business_stream, log_stream).boxed()
        } else {
            business_stream.boxed()
        };

    let combined = stream::once(async move { Ok(initial_event) }).chain(stream);

    Sse::new(combined).keep_alive(KeepAlive::default())
}
