//! Web API routes.

pub mod bundles;
pub mod chat;
pub mod clas;
pub mod config;
pub mod files;
pub mod security;
pub mod senml;
pub mod services;
pub mod static_routes;

use axum::{
    Json,
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;

/// Prepares common headers for serving payload data (Content-Type, Content-Disposition, CSP, etc.).
fn prepare_payload_headers(
    total_len: u64,
    mime_type: &str,
    filename: &str,
    is_download: bool,
) -> HeaderMap {
    // Determine safe disposition type.
    // Dangerous web types like HTML must NEVER be rendered inline without sandbox.
    // SVGs are rendered safely within <img> tags with strict sandbox CSP.
    let is_safe_inline_mime = mime_type.starts_with("image/png")
        || mime_type.starts_with("image/jpeg")
        || mime_type.starts_with("image/gif")
        || mime_type.starts_with("image/webp")
        || mime_type.starts_with("image/svg+xml")
        || mime_type.starts_with("audio/")
        || mime_type.starts_with("video/")
        || mime_type.starts_with("text/plain")
        || mime_type.starts_with("text/markdown");

    let disposition_type = if is_download || !is_safe_inline_mime {
        "attachment"
    } else {
        "inline"
    };

    // Sanitize filename against CRLF, quotes, backslashes for Content-Disposition header
    let safe_filename: String = filename
        .chars()
        .filter(|c| !matches!(c, '"' | '\r' | '\n' | '\\'))
        .collect();
    let safe_filename = if safe_filename.is_empty() {
        "payload.bin".to_string()
    } else {
        safe_filename
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("{disposition_type}; filename=\"{safe_filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("inline")),
    );
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    let _ = total_len;
    headers
}

/// Parses standard HTTP Range header (e.g. bytes=0-1023, bytes=1024-, bytes=-500).
fn parse_http_range(req_headers: &HeaderMap, total_len: u64) -> Option<Result<(u64, u64), ()>> {
    let range_header = req_headers.get(header::RANGE)?.to_str().ok()?;
    let range_spec = range_header.strip_prefix("bytes=")?;
    let first_range = range_spec.split(',').next()?.trim();
    let (start_str, end_str) = first_range.split_once('-')?;
    if total_len == 0 {
        return Some(Err(()));
    }

    if start_str.is_empty() {
        // Suffix range: -N (last N bytes)
        let suffix_len = end_str.parse::<u64>().ok()?;
        if suffix_len == 0 {
            return Some(Err(()));
        }
        let start = total_len.saturating_sub(suffix_len);
        let end = total_len - 1;
        Some(Ok((start, end)))
    } else if let Ok(start) = start_str.parse::<u64>() {
        if start < total_len {
            let end = if end_str.is_empty() {
                total_len - 1
            } else {
                end_str
                    .parse::<u64>()
                    .unwrap_or(total_len - 1)
                    .min(total_len - 1)
            };
            if start <= end {
                Some(Ok((start, end)))
            } else {
                Some(Err(()))
            }
        } else {
            Some(Err(()))
        }
    } else {
        Some(Err(()))
    }
}

/// Serves a byte payload with support for Content-Type, Content-Disposition, and HTTP Range requests (RFC 9110).
pub fn serve_payload_with_range(
    bytes: Bytes,
    raw_mime: &str,
    filename: &str,
    is_download: bool,
    req_headers: &HeaderMap,
) -> Response {
    let total_len = bytes.len() as u64;

    // Dynamically refine Ogg Opus MIME type if OpusHead signature is detected
    let mut mime_type = raw_mime.to_string();
    if (mime_type == "audio/ogg" || mime_type == "audio/opus" || mime_type.is_empty())
        && total_len >= 36
        && &bytes[0..4] == b"OggS"
    {
        let page_segments = bytes[26] as usize;
        let packet_offset = 27 + page_segments;
        if total_len as usize >= packet_offset + 8
            && &bytes[packet_offset..packet_offset + 8] == b"OpusHead"
        {
            mime_type = "audio/ogg; codecs=opus".to_string();
        }
    }

    let mut headers = prepare_payload_headers(total_len, &mime_type, filename, is_download);

    // Handle HTTP Range header
    if let Some(range_res) = parse_http_range(req_headers, total_len) {
        match range_res {
            Ok((start, end)) => {
                let range_len = end - start + 1;
                if let Ok(hv) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total_len}")) {
                    headers.insert(header::CONTENT_RANGE, hv);
                }
                headers.insert(header::CONTENT_LENGTH, HeaderValue::from(range_len));

                let sliced = bytes.slice((start as usize)..=(end as usize));
                return (StatusCode::PARTIAL_CONTENT, headers, Body::from(sliced)).into_response();
            }
            Err(()) => {
                if let Ok(hv) = HeaderValue::from_str(&format!("bytes */{total_len}")) {
                    headers.insert(header::CONTENT_RANGE, hv);
                }
                return (StatusCode::RANGE_NOT_SATISFIABLE, headers, Body::empty()).into_response();
            }
        }
    }

    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(total_len));
    (StatusCode::OK, headers, Body::from(bytes)).into_response()
}

/// Streams a file from disk with O(1) memory footprint and full HTTP Range support.
pub async fn serve_payload_file_with_range(
    path: &std::path::Path,
    raw_mime: &str,
    filename: &str,
    is_download: bool,
    req_headers: &HeaderMap,
) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    use tokio_util::io::ReaderStream;

    let metadata = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let total_len = metadata.len();
    let mut mime_type = raw_mime.to_string();

    // Check Ogg Opus header from first 64 bytes
    if (mime_type == "audio/ogg" || mime_type == "audio/opus" || mime_type.is_empty())
        && total_len >= 36
        && let Ok(mut f) = tokio::fs::File::open(path).await
    {
        let mut head = [0u8; 64];
        if let Ok(n) = f.read(&mut head).await
            && n >= 36
            && &head[0..4] == b"OggS"
        {
            let page_segments = head[26] as usize;
            let packet_offset = 27 + page_segments;
            if n >= packet_offset + 8 && &head[packet_offset..packet_offset + 8] == b"OpusHead" {
                mime_type = "audio/ogg; codecs=opus".to_string();
            }
        }
    }

    let mut headers = prepare_payload_headers(total_len, &mime_type, filename, is_download);

    if let Some(range_res) = parse_http_range(req_headers, total_len) {
        match range_res {
            Ok((start, end)) => {
                let range_len = end - start + 1;
                if let Ok(hv) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total_len}")) {
                    headers.insert(header::CONTENT_RANGE, hv);
                }
                headers.insert(header::CONTENT_LENGTH, HeaderValue::from(range_len));

                let mut file = match tokio::fs::File::open(path).await {
                    Ok(f) => f,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": e.to_string() })),
                        )
                            .into_response();
                    }
                };
                if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": e.to_string() })),
                    )
                        .into_response();
                }
                let take = file.take(range_len);
                let stream = ReaderStream::new(take);
                return (
                    StatusCode::PARTIAL_CONTENT,
                    headers,
                    Body::from_stream(stream),
                )
                    .into_response();
            }
            Err(()) => {
                if let Ok(hv) = HeaderValue::from_str(&format!("bytes */{total_len}")) {
                    headers.insert(header::CONTENT_RANGE, hv);
                }
                return (StatusCode::RANGE_NOT_SATISFIABLE, headers, Body::empty()).into_response();
            }
        }
    }

    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(total_len));
    let file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let stream = ReaderStream::new(file);
    (StatusCode::OK, headers, Body::from_stream(stream)).into_response()
}
