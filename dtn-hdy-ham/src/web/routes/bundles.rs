//! Bundle creation and injection endpoint handler.

use axum::Json;
use axum::extract::{FromRequest, Multipart, Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bytes::Bytes;
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::Eid;
use serde::Deserialize;
use serde_json::json;

use crate::web::AppState;

/// JSON payload structure for sending a bundle.
#[derive(Debug, Deserialize)]
pub struct SendBundleJsonRequest {
    pub destination: String,
    pub source: Option<String>,
    pub lifetime: Option<u64>,
    pub payload: String,
}

fn normalize_eid(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s
        .strip_prefix("dtn://")
        .filter(|r| !r.is_empty() && !r.contains('/'))
    {
        return format!("dtn://{}/", rest);
    }
    s.to_string()
}

async fn build_and_inject_bundle(
    state: &AppState,
    destination_str: &str,
    source_str: Option<&str>,
    lifetime_secs: Option<u64>,
    payload_bytes: Vec<u8>,
) -> Result<serde_json::Value, (StatusCode, String)> {
    if destination_str.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Destination EID cannot be empty".to_string(),
        ));
    }

    let norm_dest = normalize_eid(destination_str);
    let destination_eid: Eid = norm_dest.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid destination EID '{norm_dest}': {e}"),
        )
    })?;

    let source_eid: Eid = if let Some(src) = source_str.filter(|s| !s.trim().is_empty()) {
        let norm_src = normalize_eid(src);
        norm_src.parse().map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid source EID '{norm_src}': {e}"),
            )
        })?
    } else {
        let norm_src = normalize_eid(&state.config.node_id);
        norm_src.parse().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Invalid station node-id '{norm_src}': {e}"),
            )
        })?
    };

    let lifetime = std::time::Duration::from_secs(lifetime_secs.unwrap_or(3600));
    let payload_len = payload_bytes.len();

    let (bundle, binbundle) = Builder::new(source_eid.clone(), destination_eid.clone())
        .with_payload(payload_bytes.into())
        .with_lifetime(lifetime)
        .build(CreationTimestamp::now())
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to build bundle: {e}"),
            )
        })?;

    let bundle_id = bundle.primary.id.to_key();
    let bundle_bytes = Bytes::from(binbundle.into_vec());

    state.injector.inject(bundle_bytes).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to dispatch bundle to BPA: {e}"),
        )
    })?;

    Ok(json!({
        "status": "sent",
        "bundle_id": bundle_id,
        "source": source_eid.to_string(),
        "destination": destination_eid.to_string(),
        "payload_size": payload_len,
        "lifetime_secs": lifetime.as_secs(),
    }))
}

/// Handler for `POST /api/v1/bundles` supporting both JSON and multipart/form-data.
pub async fn send_bundle(State(state): State<AppState>, request: Request) -> impl IntoResponse {
    let content_type = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if content_type.starts_with("multipart/form-data") {
        let mut multipart = match Multipart::from_request(request, &state).await {
            Ok(m) => m,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Invalid multipart form data: {e}") })),
                )
                    .into_response();
            }
        };

        let mut destination: Option<String> = None;
        let mut source: Option<String> = None;
        let mut lifetime: Option<u64> = None;
        let mut payload_bytes: Option<Vec<u8>> = None;

        while let Ok(Some(field)) = multipart.next_field().await {
            let name = field.name().unwrap_or("").to_string();
            match name.as_str() {
                "destination" => {
                    if let Ok(text) = field.text().await {
                        destination = Some(text);
                    }
                }
                "source" => {
                    if let Ok(text) = field.text().await {
                        source = Some(text);
                    }
                }
                "lifetime" => {
                    if let Ok(text) = field.text().await {
                        lifetime = text.trim().parse::<u64>().ok();
                    }
                }
                "payload" | "file" => {
                    if let Ok(bytes) = field.bytes().await {
                        payload_bytes = Some(bytes.to_vec());
                    }
                }
                _ => {}
            }
        }

        let dest = destination.unwrap_or_default();
        let payload = payload_bytes.unwrap_or_default();

        match build_and_inject_bundle(&state, &dest, source.as_deref(), lifetime, payload).await {
            Ok(res) => (StatusCode::CREATED, Json(res)).into_response(),
            Err((code, err)) => (code, Json(json!({ "error": err }))).into_response(),
        }
    } else {
        let Json(req) = match Json::<SendBundleJsonRequest>::from_request(request, &state).await {
            Ok(j) => j,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Invalid JSON request: {e}") })),
                )
                    .into_response();
            }
        };

        match build_and_inject_bundle(
            &state,
            &req.destination,
            req.source.as_deref(),
            req.lifetime,
            req.payload.into_bytes(),
        )
        .await
        {
            Ok(res) => (StatusCode::CREATED, Json(res)).into_response(),
            Err((code, err)) => (code, Json(json!({ "error": err }))).into_response(),
        }
    }
}
