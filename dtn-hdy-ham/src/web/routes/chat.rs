//! REST handlers for Chat conversations and message dispatching.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::Eid;
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;
use tracing::{error, info};

use crate::chat_manager::{ChatMessage, ConversationSummary, normalize_chat_target};
use crate::web::AppState;

/// Request payload for sending a chat message.
#[derive(Debug, Deserialize)]
pub struct SendChatMessageRequest {
    pub peer: Option<String>,
    pub content: String,
    pub lifetime: Option<u64>,
}

/// Request payload for creating a persistent conversation.
#[derive(Debug, Deserialize)]
pub struct CreateConversationRequest {
    pub peer: String,
}

/// Query parameters for peer-filtered operations.
#[derive(Debug, Deserialize)]
pub struct PeerQuery {
    pub peer: String,
}

/// Request payload for marking a conversation read.
#[derive(Debug, Deserialize)]
pub struct MarkReadRequest {
    pub peer: String,
}

/// Helper to convert anyhow errors into Axum 500 responses.
fn internal_error<E: std::fmt::Display>(err: E) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": err.to_string() })),
    )
}

/// GET /api/v1/chat/conversations - Returns list of all conversation threads.
pub async fn list_conversations(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConversationSummary>>, (StatusCode, Json<serde_json::Value>)> {
    let convs = state
        .chat_manager
        .list_conversations()
        .map_err(internal_error)?;
    Ok(Json(convs))
}

/// POST /api/v1/chat/conversations - Explicitly creates and persists a new conversation thread.
pub async fn create_conversation(
    State(state): State<AppState>,
    Json(payload): Json<CreateConversationRequest>,
) -> impl IntoResponse {
    let peer = payload.peer.trim();
    if peer.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Peer cannot be empty" })),
        )
            .into_response();
    }
    match state.chat_manager.create_conversation(peer) {
        Ok(conv) => (StatusCode::CREATED, Json(conv)).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/chat/conversations/{peer} - Returns all messages for a given peer conversation.
pub async fn get_conversation_messages(
    State(state): State<AppState>,
    Path(peer): Path<String>,
) -> Result<Json<Vec<ChatMessage>>, (StatusCode, Json<serde_json::Value>)> {
    let messages = state
        .chat_manager
        .get_conversation_messages(&peer)
        .map_err(internal_error)?;
    Ok(Json(messages))
}

/// GET /api/v1/chat/messages?peer=... - Query-based message retrieval.
pub async fn get_conversation_messages_query(
    State(state): State<AppState>,
    Query(query): Query<PeerQuery>,
) -> Result<Json<Vec<ChatMessage>>, (StatusCode, Json<serde_json::Value>)> {
    let messages = state
        .chat_manager
        .get_conversation_messages(&query.peer)
        .map_err(internal_error)?;
    Ok(Json(messages))
}

/// Core dispatch logic for sending a chat message.
async fn do_send_chat_message(
    state: AppState,
    peer: &str,
    content: &str,
    lifetime_override: Option<u64>,
) -> Response {
    let content = content.trim();
    if content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Message content cannot be empty" })),
        )
            .into_response();
    }
    if peer.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Peer cannot be empty" })),
        )
            .into_response();
    }

    let target = normalize_chat_target(peer);
    let sender_str = normalize_chat_target(&state.config.node_id);

    let recipient_eid: Eid = match target.parse() {
        Ok(e) => e,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Invalid recipient EID '{target}': {err}") })),
            )
                .into_response();
        }
    };

    let sender_eid: Eid = match sender_str.parse() {
        Ok(e) => e,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Invalid sender EID '{sender_str}': {err}") })),
            )
                .into_response();
        }
    };

    // Chat message lifetime: default to 3600 seconds (1 hour) matching standard bundle lifetime and peer chat messages
    let lifetime_secs = lifetime_override.unwrap_or(3600);
    let lifetime = std::time::Duration::from_secs(lifetime_secs);

    let (bundle, binbundle) = match Builder::new(sender_eid.clone(), recipient_eid.clone())
        .with_payload(content.as_bytes().into())
        .with_lifetime(lifetime)
        .build(CreationTimestamp::now())
    {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to build chat bundle: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to build chat bundle: {e}") })),
            )
                .into_response();
        }
    };

    let dest_str = recipient_eid.to_string();
    let (binbundle_vec, auth_status) =
        if let Some(key) = state.keystore.find_outbound_key(&dest_str) {
            tracing::info!(
                "Signing chat message with BPSec BIB key pattern '{}'",
                key.pattern
            );
            match crate::security::sign_bundle_with_key(&bundle, &binbundle, &key, None) {
                Ok(bytes) => (bytes, "valid".to_string()),
                Err(e) => {
                    error!("Failed to sign chat bundle with BPSec key: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": format!("Failed to sign chat message: {e}") })),
                    )
                        .into_response();
                }
            }
        } else {
            (binbundle.into_vec(), "unsigned".to_string())
        };

    let bundle_id = bundle.primary.id.to_key();
    let bundle_bytes = Bytes::from(binbundle_vec);

    let dt = bundle
        .primary
        .id
        .timestamp
        .as_datetime()
        .unwrap_or_else(OffsetDateTime::now_utc);

    let created_at = dt
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| dt.to_string());

    let timestamp = (dt.unix_timestamp_nanos() / 1_000_000) as i64;
    let lifetime_secs = bundle.primary.lifetime.as_secs();
    let expiry_timestamp = timestamp + (lifetime_secs as i64 * 1000);

    let mut message = ChatMessage {
        id: bundle_id.clone(),
        conversation_peer: target.clone(),
        sender_eid: sender_str,
        recipient_eid: target,
        content: content.to_string(),
        direction: "outbound".to_string(),
        status: "queued".to_string(),
        is_read: true,
        is_sticky: false,
        timestamp,
        expiry_timestamp,
        created_at,
        auth_status,
    };

    info!(
        "Dispatching chat message to {} (id: {}, bytes: {})",
        recipient_eid,
        bundle_id,
        content.len()
    );

    // Inject into BPA sink
    if let Err(e) = state.injector.inject(bundle_bytes).await {
        error!("Failed to dispatch chat bundle to BPA: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to dispatch chat bundle: {e}") })),
        )
            .into_response();
    }

    // Mark as sent once injected into BPA forwarder
    message.status = "sent".to_string();

    if let Err(e) = state.chat_manager.insert_message(&message) {
        error!("Failed to store outbound chat message: {e}");
    }

    state
        .service_tracker
        .broadcast_chat_message(message.clone());

    (StatusCode::CREATED, Json(message)).into_response()
}

/// POST /api/v1/chat/conversations/{peer} - Sends a new chat message to a peer.
pub async fn send_chat_message(
    State(state): State<AppState>,
    Path(peer): Path<String>,
    Json(payload): Json<SendChatMessageRequest>,
) -> impl IntoResponse {
    do_send_chat_message(state, &peer, &payload.content, payload.lifetime).await
}

/// POST /api/v1/chat/messages - Sends a new chat message using JSON body peer & content.
pub async fn send_chat_message_body(
    State(state): State<AppState>,
    Json(payload): Json<SendChatMessageRequest>,
) -> impl IntoResponse {
    let peer = payload.peer.as_deref().unwrap_or("");
    do_send_chat_message(state, peer, &payload.content, payload.lifetime).await
}

/// POST /api/v1/chat/conversations/{peer}/read - Marks inbound messages in a conversation as read.
pub async fn mark_conversation_read(
    State(state): State<AppState>,
    Path(peer): Path<String>,
) -> impl IntoResponse {
    match state.chat_manager.mark_conversation_read(&peer) {
        Ok(count) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "marked": count })),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// POST /api/v1/chat/read - Marks inbound messages in a conversation as read via JSON body.
pub async fn mark_conversation_read_body(
    State(state): State<AppState>,
    Json(payload): Json<MarkReadRequest>,
) -> impl IntoResponse {
    match state.chat_manager.mark_conversation_read(&payload.peer) {
        Ok(count) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "marked": count })),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// POST /api/v1/chat/messages/{id}/sticky - Toggles sticky retention for a chat message.
pub async fn toggle_chat_sticky(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.internal_storage.toggle_chat_sticky(&id) {
        Ok(is_sticky) => (
            StatusCode::OK,
            Json(json!({ "id": id, "is_sticky": is_sticky })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
