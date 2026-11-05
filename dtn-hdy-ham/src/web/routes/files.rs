//! REST handlers for the `files` service (BUNDLE_LIST view).

use std::path::Path;

use crate::web::AppState;
use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::json;
use tracing::{error, info};

/// Helper for Axum 500 responses.
fn internal_error<E: std::fmt::Display>(err: E) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": err.to_string() })),
    )
}

/// GET /api/v1/files - Lists all received file bundles.
pub async fn list_files(State(state): State<AppState>) -> impl IntoResponse {
    match state.internal_storage.list_file_bundles() {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/files/{id} - Gets file bundle metadata and textual content preview.
pub async fn get_file_details(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    match state.internal_storage.get_file_bundle(&id) {
        Ok(Some(mut record)) => {
            // Attempt reading preview
            let p = Path::new(&record.payload_path);
            let mut preview = None;
            let mut file_missing = false;

            if p.exists() {
                // Generate preview if text/markdown/json up to 64 KiB max
                if record.mime_type.starts_with("text/")
                    || record.mime_type == "application/json"
                    || record.mime_type == "application/senml+json"
                {
                    use std::io::Read;
                    if let Ok(file) = std::fs::File::open(p) {
                        let mut buffer = Vec::new();
                        let mut take = file.take(64 * 1024);
                        if take.read_to_end(&mut buffer).is_ok() {
                            preview = Some(String::from_utf8_lossy(&buffer).to_string());
                        }
                    }
                }
            } else {
                file_missing = true;
            }

            // Mark read automatically on detail view
            let _ = state.internal_storage.mark_file_read(&id);
            record.is_read = true;

            (
                StatusCode::OK,
                Json(json!({
                    "bundle": record,
                    "payload_preview": preview,
                    "file_missing": file_missing,
                })),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Bundle not found" })),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct DownloadQuery {
    pub download: Option<bool>,
}

/// GET /api/v1/files/{id}/payload - Streams or downloads the raw payload file.
pub async fn download_file_payload(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    axum::extract::Query(query): axum::extract::Query<DownloadQuery>,
    req_headers: HeaderMap,
) -> impl IntoResponse {
    let record = match state.internal_storage.get_file_bundle(&id) {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Bundle not found" })),
            )
                .into_response();
        }
        Err(e) => return internal_error(e).into_response(),
    };

    let p = Path::new(&record.payload_path);
    if !p.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Payload file is missing on station storage" })),
        )
            .into_response();
    }

    let bytes = match tokio::fs::read(p).await {
        Ok(b) => bytes::Bytes::from(b),
        Err(e) => {
            error!("Failed to read payload file {}: {}", p.display(), e);
            return internal_error(e).into_response();
        }
    };

    let filename = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("dtn_payload.bin");

    super::serve_payload_with_range(
        bytes,
        &record.mime_type,
        filename,
        query.download.unwrap_or(false),
        &req_headers,
    )
}

/// POST /api/v1/files/{id}/sticky - Toggles the sticky retention flag for a file bundle.
pub async fn toggle_file_sticky(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    match state.internal_storage.toggle_file_sticky(&id) {
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

/// POST /api/v1/files/{id}/read - Marks a file bundle as read.
pub async fn mark_file_read(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    match state.internal_storage.mark_file_read(&id) {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ok", "id": id }))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// DELETE /api/v1/files/{id} - Deletes a file bundle and removes its payload.
pub async fn delete_file_bundle(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> impl IntoResponse {
    match state.internal_storage.delete_file_bundle(&id) {
        Ok(true) => {
            info!("Deleted file bundle {}", id);
            (
                StatusCode::OK,
                Json(json!({ "status": "deleted", "id": id })),
            )
                .into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Bundle not found" })),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/files/unread - Returns unread count.
pub async fn get_unread_count(State(state): State<AppState>) -> impl IntoResponse {
    match state.internal_storage.count_unread_files() {
        Ok(count) => (
            StatusCode::OK,
            Json(json!({ "unread": count, "unread_count": count })),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}
