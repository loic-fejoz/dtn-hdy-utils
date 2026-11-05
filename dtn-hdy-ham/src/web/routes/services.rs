//! Active DTN services REST and SSE event handlers.

use std::convert::Infallible;

use axum::Json;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;

use crate::service_tracker::{ServiceEvent, ServiceInfo};
use crate::web::AppState;

/// GET /api/v1/services - Returns list of active DTN services.
pub async fn get_services(State(state): State<AppState>) -> Json<Vec<ServiceInfo>> {
    Json(state.service_tracker.list_services())
}

/// GET /api/v1/events - Server-Sent Events stream for real-time station and service updates.
pub async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Send initial snapshot immediately upon connection
    let initial_services = state.service_tracker.list_services();
    let initial_json = serde_json::to_string(&initial_services).unwrap_or_default();
    let initial_event = Event::default().event("services_update").data(initial_json);

    // Stream subsequent updates from the broadcast channel
    let rx = state.service_tracker.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| async move {
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
            Err(_) => None,
        }
    });

    let combined = stream::once(async move { Ok(initial_event) }).chain(stream);

    Sse::new(combined).keep_alive(KeepAlive::default())
}
