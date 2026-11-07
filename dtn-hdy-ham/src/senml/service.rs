//! Permanent built-in DTN BPv7 service handling incoming `dtn://<station>/senml` bundles.

use bytes::Bytes;
use hardy_async::async_trait;
use hardy_bpa::services::{Result as ServiceResult, Service, ServiceSink, StatusNotify};
use hardy_bpa::stream::{Receiver, Segment, buffer_stream};
use hardy_bpv7::bundle::Id;
use hardy_bpv7::eid::Eid;
use hardy_bpv7::parse::parse;
use hardy_bpv7::status_report::ReasonCode;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::OnceCell;
use tracing::{debug, info, warn};

use super::manager::SenmlManager;
use super::parser::parse_senml_payload;
use crate::security::{AuthStatus, DirectionalKeyStore, VerifyPolicy, verify_parsed_bundle};
use crate::service_tracker::ServiceTracker;

/// Permanent DTN BPv7 service for receiving, authenticating and ingesting SenML sensor telemetry.
#[derive(Clone)]
pub struct SenmlService {
    manager: SenmlManager,
    _service_tracker: ServiceTracker,
    keystore: Arc<DirectionalKeyStore>,
    sink: Arc<OnceCell<Box<dyn ServiceSink>>>,
}

impl SenmlService {
    /// Creates a new `SenmlService` bound to manager, service tracker and keystore.
    pub fn new(
        manager: SenmlManager,
        service_tracker: ServiceTracker,
        keystore: Arc<DirectionalKeyStore>,
    ) -> Self {
        Self {
            manager,
            _service_tracker: service_tracker,
            keystore,
            sink: Arc::new(OnceCell::new()),
        }
    }

    async fn handle_delivered_bundle(&self, data: Bytes) -> ServiceResult<()> {
        let Ok(parsed) = parse(data) else {
            debug!("SenmlService: failed to parse delivered bundle");
            return Ok(());
        };

        // BPSec verification
        let auth_status = verify_parsed_bundle(&parsed, &self.keystore);
        let policy = self.keystore.get_policy();
        if policy == VerifyPolicy::Strict && auth_status == AuthStatus::Invalid {
            warn!(
                "SenmlService: dropping delivered bundle due to strict policy and invalid BPSec signature"
            );
            return Ok(());
        }

        let bundle = &parsed.bundle;
        let source_eid = &bundle.primary.id.source;

        if source_eid.is_null() || bundle.primary.flags.is_admin_record {
            debug!("SenmlService: ignoring null source or administrative bundle");
            return Ok(());
        }

        // Extract payload block (#1)
        let Some(payload_slice) = bundle
            .blocks
            .get(&1)
            .and_then(|block| block.payload(&parsed.data))
        else {
            debug!("SenmlService: delivered bundle has no payload block");
            return Ok(());
        };

        let creation_time_ms = bundle
            .primary
            .id
            .timestamp
            .as_datetime()
            .unwrap_or_else(OffsetDateTime::now_utc)
            .unix_timestamp_nanos()
            / 1_000_000;

        match parse_senml_payload(payload_slice, creation_time_ms as i64) {
            Ok(records) => {
                if records.is_empty() {
                    debug!("SenmlService: parsed empty SenML records list");
                    return Ok(());
                }

                info!(
                    "SenmlService: ingesting {} SenML records from {}",
                    records.len(),
                    source_eid
                );

                match self.manager.storage().insert_records(&records) {
                    Ok(points) => {
                        self.manager.broadcast_points(points);
                    }
                    Err(e) => {
                        warn!("SenmlService: failed to insert SenML records into SQLite: {e}");
                    }
                }
            }
            Err(e) => {
                debug!("SenmlService: failed to parse payload as SenML: {e}");
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Service for SenmlService {
    async fn on_register(&self, _source: &Eid, sink: Box<dyn ServiceSink>) {
        let _ = self.sink.set(sink);
        debug!("SenmlService registered with BPA");
    }

    async fn on_unregister(&self) {
        debug!("SenmlService unregistered from BPA");
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
        debug!("SenmlService on_deliver called for bundle_id={bundle_id:?}, len={total_len}");
        let data = buffer_stream(stream, total_len).await?;
        self.handle_delivered_bundle(data).await
    }
}
