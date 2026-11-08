//! Built-in permanent Files service handling incoming `dtn://<station>/files` bundles.
//!
//! Stores raw received files on disk and saves bundle metadata in `InternalStorage`.

use std::sync::Arc;

use bytes::Bytes;
use hardy_async::async_trait;
use hardy_bpa::services::{Result as ServiceResult, Service, ServiceSink, StatusNotify};
use hardy_bpa::stream::{Receiver, Segment, buffer_stream};
use hardy_bpv7::bundle::Id;
use hardy_bpv7::eid::Eid;
use hardy_bpv7::parse::parse;
use hardy_bpv7::status_report::ReasonCode;
use time::OffsetDateTime;
use tokio::sync::OnceCell;
use tracing::{debug, error, info, warn};

use crate::internal_storage::{FileBundleRecord, InternalStorage};
use crate::security::{AuthStatus, DirectionalKeyStore, VerifyPolicy, verify_parsed_bundle};
use crate::service_tracker::ServiceTracker;

/// Permanent DTN BPv7 service for file transfers and bundle inspection (`BUNDLE_LIST`).
#[derive(Clone)]
pub struct FilesService {
    storage: InternalStorage,
    service_tracker: ServiceTracker,
    keystore: Arc<DirectionalKeyStore>,
    sink: Arc<OnceCell<Box<dyn ServiceSink>>>,
}

impl FilesService {
    /// Creates a new `FilesService` bound to station internal storage, service tracker and keystore.
    pub fn new(
        storage: InternalStorage,
        service_tracker: ServiceTracker,
        keystore: Arc<DirectionalKeyStore>,
    ) -> Self {
        Self {
            storage,
            service_tracker,
            keystore,
            sink: Arc::new(OnceCell::new()),
        }
    }

    /// Handles a delivered bundle payload stream for the `files` service.
    async fn handle_delivered_bundle(&self, data: Bytes) -> ServiceResult<()> {
        let Ok(parsed) = parse(data) else {
            debug!("FilesService: failed to parse delivered bundle");
            return Ok(());
        };

        let auth_status = verify_parsed_bundle(&parsed, &self.keystore);
        let policy = self.keystore.get_policy();
        if policy == VerifyPolicy::Strict && auth_status == AuthStatus::Invalid {
            warn!(
                "FilesService: dropping delivered bundle due to strict policy and invalid signature"
            );
            return Ok(());
        }

        let bundle = &parsed.bundle;
        let source_eid = &bundle.primary.id.source;
        let dest_eid = &bundle.primary.destination;

        // Skip null source or administrative records
        if source_eid.is_null() || bundle.primary.flags.is_admin_record {
            debug!("FilesService: ignoring null source or admin bundle");
            return Ok(());
        }

        // Extract payload block (#1)
        let Some(payload_slice) = bundle
            .blocks
            .get(&1)
            .and_then(|block| block.payload(&parsed.data))
        else {
            debug!("FilesService: delivered bundle has no payload block");
            return Ok(());
        };

        let bundle_id = bundle.primary.id.to_key();
        let dt = bundle
            .primary
            .id
            .timestamp
            .as_datetime()
            .unwrap_or_else(OffsetDateTime::now_utc);

        let created_at = dt
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| dt.to_string());

        let creation_timestamp = (dt.unix_timestamp_nanos() / 1_000_000) as i64;
        let lifetime_secs = bundle.primary.lifetime.as_secs();
        let expiry_timestamp = creation_timestamp + (lifetime_secs as i64 * 1000);

        let (inferred_mime, _) = crate::bundle_inspector::infer_mime_type(payload_slice);
        let mime_type = inferred_mime.to_string();

        // Determine file extension from mime type
        let ext = match mime_type.as_str() {
            "text/markdown" => "md",
            "text/plain" => "txt",
            "application/json" => "json",
            "application/senml+json" => "senml.json",
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            "video/x-matroska" => "mkv",
            "video/ogg" => "ogv",
            "audio/mp4" => "m4a",
            "audio/wav" => "wav",
            "audio/mpeg" => "mp3",
            s if s.starts_with("audio/ogg") => "ogg",
            s if s.starts_with("audio/webm") => "webm",
            _ => "bin",
        };

        // Sanitize bundle id for filename
        let safe_name: String = bundle_id
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        let filename = format!("{safe_name}.{ext}");
        let payload_path = self.storage.files_dir().join(&filename);

        // Ensure storage capacity under quota policy before saving to disk
        if let Err(e) = self.storage.ensure_capacity(payload_slice.len() as u64) {
            warn!(
                "FilesService: storage quota exceeded for incoming bundle {}: {}",
                bundle_id, e
            );
            return Ok(());
        }

        // Save payload file to disk with graceful error handling (disk full / permission denied)
        if let Err(e) = std::fs::write(&payload_path, payload_slice) {
            error!(
                "FilesService: failed to write payload file to {}: {}",
                payload_path.display(),
                e
            );
            return Ok(());
        }

        let record = FileBundleRecord {
            id: bundle_id.clone(),
            source_eid: source_eid.to_string(),
            destination_eid: dest_eid.to_string(),
            creation_timestamp,
            lifetime_secs,
            expiry_timestamp,
            size: payload_slice.len() as u64,
            payload_path: payload_path.to_string_lossy().to_string(),
            mime_type,
            is_sticky: false,
            is_read: false,
            created_at,
            auth_status: auth_status.as_str().to_string(),
        };

        if let Err(e) = self.storage.insert_file_bundle(&record) {
            warn!(
                "FilesService: failed to insert bundle record {}: {}",
                bundle_id, e
            );
        } else {
            info!(
                "Stored new file bundle from {} (id: {}, bytes: {})",
                source_eid,
                bundle_id,
                payload_slice.len()
            );
            self.service_tracker.broadcast_bundles_update();
            self.service_tracker.broadcast_file_received(&record);
        }

        Ok(())
    }
}

#[async_trait]
impl Service for FilesService {
    async fn on_register(&self, _source: &Eid, sink: Box<dyn ServiceSink>) {
        let _ = self.sink.set(sink);
        debug!("FilesService registered with BPA");
    }

    async fn on_unregister(&self) {
        debug!("FilesService unregistered from BPA");
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &Id,
        _from: &Eid,
        _kind: StatusNotify,
        _reason: ReasonCode,
        _timestamp: Option<OffsetDateTime>,
    ) {
    }

    async fn on_deliver(
        &self,
        bundle_id: &Id,
        _expiry: OffsetDateTime,
        total_len: u64,
        stream: &mut dyn Receiver<Segment>,
    ) -> ServiceResult<()> {
        debug!("FilesService on_deliver called for bundle_id={bundle_id:?}, len={total_len}");
        let data = buffer_stream(stream, total_len).await?;
        self.handle_delivered_bundle(data).await
    }
}
