//! Web API routes.

pub mod bundles;
pub mod chat;
pub mod clas;
pub mod config;
pub mod files;
pub mod services;
pub mod static_routes;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;

/// Serves a byte payload with support for Content-Type, Content-Disposition, and HTTP Range requests (RFC 9110).
pub fn serve_payload_with_range(
    bytes: Bytes,
    raw_mime: &str,
    filename: &str,
    is_download: bool,
    req_headers: &HeaderMap,
) -> Response {
    let total_len = bytes.len();

    // Dynamically refine Ogg Opus MIME type if OpusHead signature is detected
    let mut mime_type = raw_mime.to_string();
    if (mime_type == "audio/ogg" || mime_type == "audio/opus" || mime_type.is_empty())
        && total_len >= 36
        && &bytes[0..4] == b"OggS"
    {
        let page_segments = bytes[26] as usize;
        let packet_offset = 27 + page_segments;
        if total_len >= packet_offset + 8 && &bytes[packet_offset..packet_offset + 8] == b"OpusHead"
        {
            mime_type = "audio/ogg; codecs=opus".to_string();
        }
    }

    // Determine safe disposition type.
    // Dangerous web types like HTML or SVG must NEVER be rendered inline without sandbox.
    let is_safe_inline_mime = mime_type.starts_with("image/png")
        || mime_type.starts_with("image/jpeg")
        || mime_type.starts_with("image/gif")
        || mime_type.starts_with("image/webp")
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
        HeaderValue::from_str(&mime_type)
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

    // Handle HTTP Range header (e.g., bytes=0-1023, bytes=1024-, bytes=-500)
    if let Some(range_header) = req_headers.get(header::RANGE).and_then(|v| v.to_str().ok())
        && let Some(range_spec) = range_header.strip_prefix("bytes=")
    {
        let first_range = range_spec.split(',').next().unwrap_or("").trim();
        if let Some((start_str, end_str)) = first_range.split_once('-')
            && total_len > 0
        {
            let range_opt: Option<(usize, usize)> = if start_str.is_empty() {
                // Suffix range: -N (last N bytes)
                end_str.parse::<usize>().ok().and_then(|suffix_len| {
                    if suffix_len == 0 {
                        None
                    } else {
                        let start = total_len.saturating_sub(suffix_len);
                        let end = total_len - 1;
                        Some((start, end))
                    }
                })
            } else if let Ok(start) = start_str.parse::<usize>() {
                if start < total_len {
                    let end = if end_str.is_empty() {
                        total_len - 1
                    } else {
                        end_str
                            .parse::<usize>()
                            .unwrap_or(total_len - 1)
                            .min(total_len - 1)
                    };
                    if start <= end {
                        Some((start, end))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            match range_opt {
                Some((start, end)) => {
                    let range_len = end - start + 1;
                    if let Ok(hv) =
                        HeaderValue::from_str(&format!("bytes {start}-{end}/{total_len}"))
                    {
                        headers.insert(header::CONTENT_RANGE, hv);
                    }
                    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(range_len));

                    let sliced = bytes.slice(start..=end);
                    return (StatusCode::PARTIAL_CONTENT, headers, Body::from(sliced))
                        .into_response();
                }
                None => {
                    if let Ok(hv) = HeaderValue::from_str(&format!("bytes */{total_len}")) {
                        headers.insert(header::CONTENT_RANGE, hv);
                    }
                    return (StatusCode::RANGE_NOT_SATISFIABLE, headers, Body::empty())
                        .into_response();
                }
            }
        }
    }

    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(total_len));
    (StatusCode::OK, headers, Body::from(bytes)).into_response()
}
