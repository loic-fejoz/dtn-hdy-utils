//! Bundle storage inspector and payload MIME inferrer for `dtn-hdy-ham`.

use hardy_bpv7::bundle::Id as Bpv7Id;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Verifies that a storage filename is safe and contains no path traversal components.
pub fn safe_payload_path(payload_dir: &Path, storage_name: &str) -> anyhow::Result<PathBuf> {
    if storage_name.contains('\\') {
        anyhow::bail!(
            "Invalid storage_name path traversal attempt: {:?}",
            storage_name
        );
    }
    let name_path = Path::new(storage_name);
    if name_path.is_absolute() {
        anyhow::bail!(
            "Invalid storage_name path traversal attempt: {:?}",
            storage_name
        );
    }
    for component in name_path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => anyhow::bail!(
                "Invalid storage_name path traversal attempt: {:?}",
                storage_name
            ),
        }
    }
    let target = payload_dir.join(name_path);
    Ok(target)
}

/// Summary metadata of a stored BPv7 bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleSummary {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub creation_time: u64,
    pub created_at: String,
    pub lifetime_secs: u64,
    pub size: usize,
    pub status: String,
    #[serde(default = "default_auth_status")]
    pub auth_status: String,
}

fn default_auth_status() -> String {
    "unsigned".to_string()
}

/// Detailed metadata of a stored BPv7 bundle including payload preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleDetail {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub creation_time: u64,
    pub created_at: String,
    pub lifetime_secs: u64,
    pub size: usize,
    pub status: String,
    pub mime_type: String,
    pub is_binary: bool,
    pub payload_size: usize,
    pub storage_name: Option<String>,
    pub payload_preview: Option<String>,
    pub hex_dump: Option<String>,
    #[serde(default = "default_auth_status")]
    pub auth_status: String,
}

/// Infers the MIME type and whether a payload is binary or textual.
#[must_use]
pub fn infer_mime_type(data: &[u8]) -> (&'static str, bool) {
    if data.is_empty() {
        return ("text/plain", false);
    }

    // Image signatures
    if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return ("image/png", true);
    }
    if data.len() >= 3 && &data[0..3] == b"\xFF\xD8\xFF" {
        return ("image/jpeg", true);
    }
    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return ("image/gif", true);
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return ("image/webp", true);
    }

    // Video signatures
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        let brand = &data[8..12];
        if brand == b"M4A " || brand == b"m4a " || brand == b"M4B " || brand == b"f4a " {
            return ("audio/mp4", true);
        }
        return ("video/mp4", true);
    }
    if data.len() >= 4 && &data[0..4] == b"\x1A\x45\xDF\xA3" {
        let search_len = data.len().min(128);
        if data[..search_len].windows(4).any(|w| w == b"webm") {
            return ("video/webm", true);
        }
        if data[..search_len].windows(8).any(|w| w == b"matroska") {
            return ("video/x-matroska", true);
        }
        return ("video/webm", true);
    }

    // Audio signatures
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return ("audio/wav", true);
    }
    if data.len() >= 4 && &data[0..4] == b"OggS" {
        if data.len() >= 36 {
            let page_segments = data[26] as usize;
            let packet_offset = 27 + page_segments;
            if data.len() >= packet_offset + 8
                && &data[packet_offset..packet_offset + 8] == b"OpusHead"
            {
                return ("audio/ogg; codecs=opus", true);
            }
            if data.len() >= packet_offset + 7
                && &data[packet_offset..packet_offset + 7] == b"\x80theora"
            {
                return ("video/ogg", true);
            }
        }
        return ("audio/ogg", true);
    }
    if (data.len() >= 3 && &data[0..3] == b"ID3")
        || (data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0)
    {
        return ("audio/mpeg", true);
    }

    // Check if valid UTF-8 text
    if let Ok(text) = std::str::from_utf8(data) {
        let trimmed = text.trim_start();
        if trimmed.starts_with("<svg")
            || ((trimmed.starts_with("<?xml") || trimmed.starts_with("<!DOCTYPE"))
                && trimmed.contains("<svg"))
        {
            return ("image/svg+xml", false);
        }

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
            let is_senml = if let Some(arr) = val.as_array() {
                arr.iter().any(|item| {
                    item.get("n").is_some() || item.get("v").is_some() || item.get("bn").is_some()
                })
            } else if let Some(obj) = val.as_object() {
                obj.contains_key("bn") || obj.contains_key("n") || obj.contains_key("e")
            } else {
                false
            };

            if is_senml {
                return ("application/senml+json", false);
            }
            return ("application/json", false);
        }

        // Markdown heuristic
        if text.contains('#') || text.contains('*') || text.contains('[') || text.contains('`') {
            return ("text/markdown", false);
        }
        return ("text/plain", false);
    }

    ("application/octet-stream", true)
}

/// Generates a standard formatted hex dump string from binary data.
#[must_use]
pub fn format_hex_dump(data: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:08x}  ", i * 16);

        // Hex bytes
        for (j, byte) in chunk.iter().enumerate() {
            if j == 8 {
                out.push(' ');
            }
            let _ = write!(&mut out, "{:02x} ", byte);
        }

        // Padding if last chunk < 16 bytes
        if chunk.len() < 16 {
            let missing = 16 - chunk.len();
            for j in 0..missing {
                if chunk.len() + j == 8 {
                    out.push(' ');
                }
                out.push_str("   ");
            }
        }

        out.push_str(" |");
        for byte in chunk {
            if byte.is_ascii_graphic() || *byte == b' ' {
                out.push(*byte as char);
            } else {
                out.push('.');
            }
        }
        out.push_str("|\n");
    }
    out
}

/// Lists all stored bundles from the SQLite metadata database.
pub fn list_bundles_from_db(
    db_path: &Path,
    payload_dir: Option<&Path>,
    keystore: Option<&crate::security::DirectionalKeyStore>,
) -> anyhow::Result<Vec<BundleSummary>> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let conn = Connection::open(db_path)?;
    let mut stmt =
        conn.prepare("SELECT bundle_id, bundle, status_code FROM bundles ORDER BY rowid DESC")?;

    let rows = stmt.query_map([], |row| {
        let id_bytes: Vec<u8> = row.get(0)?;
        let bundle_bytes: Option<Vec<u8>> = row.get(1)?;
        let status_code: Option<i64> = row.get(2)?;
        Ok((id_bytes, bundle_bytes, status_code))
    })?;

    let mut list = Vec::new();
    for r in rows {
        let (id_bytes, bundle_bytes, status_code) = match r {
            Ok(v) => v,
            Err(e) => {
                debug!("Failed to read bundle row: {e}");
                continue;
            }
        };

        let bpv7_id: Bpv7Id = match serde_json::from_slice(&id_bytes) {
            Ok(id) => id,
            Err(_) => continue,
        };

        let (dest, lifetime_secs, size, status, auth_status) = if let Some(bytes) = bundle_bytes {
            let bundle_json: serde_json::Value = match serde_json::from_slice(&bytes) {
                Ok(j) => j,
                Err(_) => continue,
            };

            let dest = bundle_json["bpv7"]["primary"]["destination"]
                .as_str()
                .unwrap_or("dtn:none")
                .to_string();

            let lifetime_secs = bundle_json["bpv7"]["primary"]["lifetime"]
                .as_u64()
                .unwrap_or(3600);

            let size = bundle_json["bpv7"]["blocks"]
                .as_object()
                .map(|blocks| {
                    blocks
                        .values()
                        .filter_map(|b| b["extent"]["end"].as_u64())
                        .max()
                        .unwrap_or(0) as usize
                })
                .unwrap_or(0);

            let status = match status_code.unwrap_or(0) {
                0 => "New",
                1 => "Waiting",
                2 => "ForwardPending",
                3 => "AduFragment",
                4 => "Dispatching",
                5 => "WaitingForService",
                _ => "Stored",
            };

            let has_bib = bundle_json["bpv7"]["blocks"]
                .as_object()
                .map(|blocks| {
                    blocks.values().any(|b| {
                        b["block_type"].as_u64() == Some(11)
                            || b["type"].as_str() == Some("BlockIntegrity")
                    })
                })
                .unwrap_or(false);

            let auth_status = if !has_bib {
                "unsigned".to_string()
            } else if let Some(ks) = keystore {
                if ks.get_policy() == crate::security::VerifyPolicy::Ignore {
                    "unsigned".to_string()
                } else {
                    let storage_name = bundle_json["metadata"]["storage_name"]
                        .as_str()
                        .map(|s| s.to_string());
                    let verified = if let (Some(name), Some(dir)) = (storage_name, payload_dir) {
                        if let Ok(path) = safe_payload_path(dir, &name) {
                            if let Ok(raw_bytes) = std::fs::read(&path) {
                                Some(crate::security::verify_bundle(&raw_bytes, ks))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    match verified {
                        Some(status) => status.as_str().to_string(),
                        None => {
                            let src = bpv7_id.source.to_string();
                            if ks.find_inbound_key(&src).is_none() {
                                "missing_key".to_string()
                            } else {
                                "valid".to_string()
                            }
                        }
                    }
                }
            } else {
                "unsigned".to_string()
            };

            (dest, lifetime_secs, size, status.to_string(), auth_status)
        } else {
            (
                "-".to_string(),
                3600,
                0,
                "Completed".to_string(),
                "unsigned".to_string(),
            )
        };

        let created_at = bpv7_id
            .timestamp
            .as_datetime()
            .map(|dt| {
                dt.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| dt.to_string())
            })
            .unwrap_or_else(|| "Clockless".to_string());

        let creation_time = bpv7_id
            .timestamp
            .creation_time()
            .map_or(0, |t| t.millisecs());

        list.push(BundleSummary {
            id: bpv7_id.to_key(),
            source: bpv7_id.source.to_string(),
            destination: dest,
            creation_time,
            created_at,
            lifetime_secs,
            size,
            status,
            auth_status,
        });
    }

    Ok(list)
}

/// Looks up a bundle's raw ID bytes and optional storage filename synchronously.
pub fn lookup_bundle_storage_info(
    db_path: &Path,
    bundle_key: &str,
) -> anyhow::Result<Option<(Vec<u8>, Option<String>)>> {
    if !db_path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT bundle_id, bundle FROM bundles ORDER BY rowid DESC")?;

    let rows = stmt.query_map([], |row| {
        let id_bytes: Vec<u8> = row.get(0)?;
        let bundle_bytes: Option<Vec<u8>> = row.get(1)?;
        Ok((id_bytes, bundle_bytes))
    })?;

    for r in rows {
        let (id_bytes, bundle_bytes_opt) = match r {
            Ok(v) => v,
            Err(_) => continue,
        };
        let bpv7_id: Bpv7Id = match serde_json::from_slice(&id_bytes) {
            Ok(id) => id,
            Err(_) => continue,
        };

        if bpv7_id.to_key() == bundle_key {
            let storage_name = if let Some(bytes) = bundle_bytes_opt {
                let bundle_json: serde_json::Value =
                    serde_json::from_slice(&bytes).unwrap_or_default();
                bundle_json["metadata"]["storage_name"]
                    .as_str()
                    .map(|s| s.to_string())
            } else {
                None
            };
            return Ok(Some((id_bytes, storage_name)));
        }
    }

    Ok(None)
}

/// Deletes a bundle row by ID from SQLite metadata database.
pub fn delete_bundle_db_record(db_path: &Path, id_bytes: &[u8]) -> anyhow::Result<bool> {
    if !db_path.exists() {
        return Ok(false);
    }
    let conn = Connection::open(db_path)?;
    let deleted = conn.execute("DELETE FROM bundles WHERE bundle_id = ?", params![id_bytes])?;
    Ok(deleted > 0)
}

/// Fetches the raw extracted payload bytes and its detected MIME type for a given bundle key.
pub async fn get_bundle_payload(
    db_path: &Path,
    payload_dir: &Path,
    bundle_key: &str,
) -> anyhow::Result<Option<(Vec<u8>, &'static str)>> {
    let Some((_id_bytes, storage_name)) = lookup_bundle_storage_info(db_path, bundle_key)? else {
        return Ok(None);
    };

    let Some(name) = storage_name else {
        return Ok(None);
    };

    let payload_path = safe_payload_path(payload_dir, &name)?;
    if !payload_path.exists() {
        return Ok(None);
    }

    let raw_bytes = tokio::fs::read(&payload_path).await?;
    if let Ok(parsed) = hardy_bpv7::parse::parse(raw_bytes.clone().into())
        && let Some(p_block) = parsed.bundle.blocks.get(&1)
        && let Some(payload_slice) = p_block.payload(&parsed.data)
    {
        let (mime, _) = infer_mime_type(payload_slice);
        return Ok(Some((payload_slice.to_vec(), mime)));
    }

    let (mime, _) = infer_mime_type(&raw_bytes);
    Ok(Some((raw_bytes, mime)))
}

/// Retrieves detailed bundle metadata, MIME type, and formatted preview.
pub async fn get_bundle_detail(
    db_path: &Path,
    payload_dir: &Path,
    bundle_key: &str,
    keystore: Option<&crate::security::DirectionalKeyStore>,
) -> anyhow::Result<Option<BundleDetail>> {
    if !db_path.exists() {
        return Ok(None);
    }

    let summaries = list_bundles_from_db(db_path, Some(payload_dir), keystore)?;
    let Some(summary) = summaries.into_iter().find(|s| s.id == bundle_key) else {
        return Ok(None);
    };

    let storage_name = lookup_bundle_storage_info(db_path, bundle_key)?.and_then(|(_, name)| name);

    let (payload_bytes, _) =
        if let Some((bytes, mime)) = get_bundle_payload(db_path, payload_dir, bundle_key).await? {
            (bytes, mime)
        } else {
            (Vec::new(), "text/plain")
        };

    let (mime_type, is_binary) = if summary.status == "Completed" {
        ("text/plain", false)
    } else {
        infer_mime_type(&payload_bytes)
    };

    let payload_preview = if summary.status == "Completed" {
        Some(
            "[Payload released: bundle forwarding or local delivery completed successfully]"
                .to_string(),
        )
    } else if !is_binary {
        const MAX_PREVIEW_BYTES: usize = 16 * 1024;
        let preview_slice = &payload_bytes[..payload_bytes.len().min(MAX_PREVIEW_BYTES)];
        String::from_utf8(preview_slice.to_vec()).ok()
    } else {
        None
    };

    let hex_dump = if is_binary && !payload_bytes.is_empty() {
        const MAX_HEX_BYTES: usize = 4 * 1024;
        let hex_slice = &payload_bytes[..payload_bytes.len().min(MAX_HEX_BYTES)];
        Some(format_hex_dump(hex_slice))
    } else {
        None
    };

    Ok(Some(BundleDetail {
        id: summary.id,
        source: summary.source,
        destination: summary.destination,
        creation_time: summary.creation_time,
        created_at: summary.created_at,
        lifetime_secs: summary.lifetime_secs,
        size: summary.size,
        status: summary.status,
        mime_type: mime_type.to_string(),
        is_binary,
        payload_size: payload_bytes.len(),
        storage_name,
        payload_preview,
        hex_dump,
        auth_status: summary.auth_status,
    }))
}

/// Deletes a bundle and its payload file from storage.
pub async fn delete_bundle_from_storage(
    db_path: &Path,
    payload_dir: &Path,
    bundle_key: &str,
) -> anyhow::Result<bool> {
    let Some((id_bytes, storage_name)) = lookup_bundle_storage_info(db_path, bundle_key)? else {
        return Ok(false);
    };

    // Delete payload file if present
    if let Some(ref name) = storage_name
        && let Ok(payload_file) = safe_payload_path(payload_dir, name)
        && payload_file.exists()
    {
        let _ = tokio::fs::remove_file(payload_file).await;
    }

    // Delete row from SQLite
    delete_bundle_db_record(db_path, &id_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_payload_path() {
        let base = Path::new("/tmp/payloads");
        assert!(safe_payload_path(base, "bundle_123.bin").is_ok());
        assert!(safe_payload_path(base, "70/be/40352f2a").is_ok());
        assert_eq!(
            safe_payload_path(base, "70/be/40352f2a").unwrap(),
            base.join("70/be/40352f2a")
        );

        assert!(safe_payload_path(base, "../etc/passwd").is_err());
        assert!(safe_payload_path(base, "/etc/passwd").is_err());
        assert!(safe_payload_path(base, "70/../passwd").is_err());
        assert!(safe_payload_path(base, "..\\windows\\system32").is_err());
        assert!(safe_payload_path(base, ".").is_err());
        assert!(safe_payload_path(base, "..").is_err());
    }

    #[test]
    fn test_infer_mime_type_video() {
        let mp4 = b"\x00\x00\x00\x20ftypisom\x00\x00\x02\x00isomiso2mp41";
        assert_eq!(infer_mime_type(mp4), ("video/mp4", true));

        let m4a = b"\x00\x00\x00\x20ftypM4A \x00\x00\x00\x00M4A mp42isom";
        assert_eq!(infer_mime_type(m4a), ("audio/mp4", true));

        let webm = b"\x1A\x45\xDF\xA3\x9F\x42\x86\x81\x01\x42\xF7\x81\x01\x42\xF2\x81\x04\x42\xF3\x81\x08\x42\x82\x84webm";
        assert_eq!(infer_mime_type(webm), ("video/webm", true));
    }

    #[test]
    fn test_infer_mime_type_images() {
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert_eq!(infer_mime_type(png), ("image/png", true));

        let jpeg = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        assert_eq!(infer_mime_type(jpeg), ("image/jpeg", true));

        let gif = b"GIF89a\x01\x00\x01\x00";
        assert_eq!(infer_mime_type(gif), ("image/gif", true));

        let webp = b"RIFF\x14\x00\x00\x00WEBPVP8 ";
        assert_eq!(infer_mime_type(webp), ("image/webp", true));

        let svg_direct = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=\"10\"/></svg>";
        assert_eq!(infer_mime_type(svg_direct), ("image/svg+xml", false));

        let svg_xml =
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg viewBox=\"0 0 100 100\"></svg>";
        assert_eq!(infer_mime_type(svg_xml), ("image/svg+xml", false));
    }

    #[test]
    fn test_infer_mime_type_audio() {
        let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ";
        assert_eq!(infer_mime_type(wav), ("audio/wav", true));

        let ogg = b"OggS\x00\x02\x00\x00";
        assert_eq!(infer_mime_type(ogg), ("audio/ogg", true));

        let mp3_id3 = b"ID3\x03\x00\x00\x00";
        assert_eq!(infer_mime_type(mp3_id3), ("audio/mpeg", true));

        let mp3_sync = b"\xFF\xFB\x90\x64\x00";
        assert_eq!(infer_mime_type(mp3_sync), ("audio/mpeg", true));
    }

    #[test]
    fn test_infer_mime_type_senml_and_json() {
        let senml = r#"[{"bn":"urn:dev:mac:0024e400000451e6/","n":"temp","v":23.1}]"#;
        assert_eq!(
            infer_mime_type(senml.as_bytes()),
            ("application/senml+json", false)
        );

        let generic_json = r#"{"station":"F4JXQ","active":true}"#;
        assert_eq!(
            infer_mime_type(generic_json.as_bytes()),
            ("application/json", false)
        );
    }

    #[test]
    fn test_infer_mime_type_markdown_and_text() {
        let md = "# Emergency DTN Bulletin\n\n* Priority: High\n* Frequency: 144.800 MHz";
        assert_eq!(infer_mime_type(md.as_bytes()), ("text/markdown", false));

        let plain = "Hello world from DTN station F4JXQ";
        assert_eq!(infer_mime_type(plain.as_bytes()), ("text/plain", false));

        let binary = b"\x00\x01\x02\x03\xFF\xFE\xFD";
        assert_eq!(infer_mime_type(binary), ("application/octet-stream", true));
    }

    #[test]
    fn test_format_hex_dump() {
        let data = b"Hello, DTN BPv7 world!\x00\x01\x02";
        let dump = format_hex_dump(data);
        assert!(dump.starts_with("00000000  48 65 6c 6c 6f 2c 20 44  54 4e 20 42 50 76 37 20"));
        assert!(dump.contains("|Hello, DTN BPv7 |"));
        assert!(dump.contains("00000010"));
    }
}
