//! Built-in permanent Chat service handling incoming `dtn://<station>/chat` bundles.

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
use tracing::{debug, info, warn};

use crate::chat_manager::{ChatManager, ChatMessage, normalize_chat_target};
use crate::service_tracker::ServiceTracker;

/// Permanent DTN BPv7 service for text messaging and chat conversations.
#[derive(Clone)]
pub struct ChatService {
    chat_manager: ChatManager,
    service_tracker: ServiceTracker,
    sink: Arc<OnceCell<Box<dyn ServiceSink>>>,
}

impl ChatService {
    /// Creates a new Chat service bound to a chat storage manager and service tracker.
    pub fn new(chat_manager: ChatManager, service_tracker: ServiceTracker) -> Self {
        Self {
            chat_manager,
            service_tracker,
            sink: Arc::new(OnceCell::new()),
        }
    }

    async fn handle_delivered_bundle(&self, data: Bytes) -> ServiceResult<()> {
        let Ok(parsed) = parse(data) else {
            debug!("ChatService: failed to parse delivered bundle");
            return Ok(());
        };

        let bundle = &parsed.bundle;
        let source_eid = &bundle.primary.id.source;
        let dest_eid = &bundle.primary.destination;

        // Skip null source or administrative records
        if source_eid.is_null() || bundle.primary.flags.is_admin_record {
            debug!("ChatService: ignoring null source or admin bundle");
            return Ok(());
        }

        // Extract payload block (#1)
        let Some(payload_slice) = bundle
            .blocks
            .get(&1)
            .and_then(|block| block.payload(&parsed.data))
        else {
            debug!("ChatService: delivered bundle has no payload block");
            return Ok(());
        };

        let content = String::from_utf8_lossy(payload_slice).to_string();
        let bundle_id = bundle.primary.id.to_key();
        let conversation_peer = normalize_chat_target(&source_eid.to_string());

        let dt = bundle
            .primary
            .id
            .timestamp
            .as_datetime()
            .unwrap_or_else(OffsetDateTime::now_utc);

        let created_at = dt
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| dt.to_string());

        let creation_time_ms = (dt.unix_timestamp_nanos() / 1_000_000) as i64;
        let lifetime_secs = bundle.primary.lifetime.as_secs();
        let expiry_timestamp = creation_time_ms + (lifetime_secs as i64 * 1000);

        let msg = ChatMessage {
            id: bundle_id.clone(),
            conversation_peer,
            sender_eid: source_eid.to_string(),
            recipient_eid: dest_eid.to_string(),
            content,
            direction: "inbound".to_string(),
            status: "received".to_string(),
            is_read: false,
            is_sticky: false,
            timestamp: creation_time_ms,
            expiry_timestamp,
            created_at,
        };

        if let Err(e) = self.chat_manager.insert_message(&msg) {
            warn!("Failed to store inbound chat message {}: {}", bundle_id, e);
        } else {
            info!(
                "Stored new inbound chat message from {} (id: {})",
                source_eid, bundle_id
            );
            self.service_tracker.broadcast_chat_message(msg);
        }

        Ok(())
    }
}

#[async_trait]
impl Service for ChatService {
    async fn on_register(&self, _source: &Eid, sink: Box<dyn ServiceSink>) {
        let _ = self.sink.set(sink);
        debug!("ChatService registered with BPA");
    }

    async fn on_unregister(&self) {
        debug!("ChatService unregistered from BPA");
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &Id,
        _from: &Eid,
        _kind: StatusNotify,
        _reason: ReasonCode,
        _timestamp: Option<OffsetDateTime>,
    ) {
        // Chat service does not inspect raw status reports
    }

    async fn on_deliver(
        &self,
        bundle_id: &Id,
        _expiry: OffsetDateTime,
        total_len: u64,
        stream: &mut dyn Receiver<Segment>,
    ) -> ServiceResult<()> {
        debug!("ChatService on_deliver called for bundle_id={bundle_id:?}, len={total_len}");
        let data = buffer_stream(stream, total_len).await?;
        self.handle_delivered_bundle(data).await
    }
}
