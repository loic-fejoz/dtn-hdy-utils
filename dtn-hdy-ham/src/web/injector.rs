//! In-process CLA adapter to inject bundles created by the web server into Hardy BPA.

use std::num::NonZeroU64;
use std::sync::Arc;

use bytes::Bytes;
use hardy_bpa::async_trait;
use hardy_bpa::cla::{Cla, ClaAddress, ForwardBundleResult, Result as ClaResult, Segment, Sink};
use hardy_bpa::stream::Receiver;
use hardy_bpv7::bundle::Id;
use hardy_bpv7::eid::NodeId;
use tokio::sync::RwLock;

/// CLA that provides bundle injection capabilities for the web interface.
#[derive(Clone, Default)]
pub struct WebBundleInjector {
    sink: Arc<RwLock<Option<Arc<dyn Sink>>>>,
}

impl WebBundleInjector {
    /// Creates a new unattached injector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sink: Arc::new(RwLock::new(None)),
        }
    }

    /// Injects an encoded bundle byte stream directly into the BPA sink.
    pub async fn inject(&self, mut bundle_bytes: Bytes) -> anyhow::Result<()> {
        let sink_opt = {
            let guard = self.sink.read().await;
            guard.clone()
        };

        if let Some(sink) = sink_opt {
            tracing::debug!(
                "WebBundleInjector: dispatching {} bytes into BPA sink",
                bundle_bytes.len()
            );
            let acceptance = sink
                .dispatch(None, None, &mut bundle_bytes)
                .await
                .map_err(|e| anyhow::anyhow!("BPA dispatch failed: {e}"))?;
            tracing::debug!(
                "WebBundleInjector: BPA acceptance verdict: {:?}",
                acceptance
            );
            Ok(())
        } else {
            anyhow::bail!("Web bundle injector sink not attached")
        }
    }
}

#[async_trait]
impl Cla for WebBundleInjector {
    async fn on_register(
        &self,
        sink: Box<dyn Sink>,
        _node_ids: &[NodeId],
        _max_bundle_size: Option<NonZeroU64>,
    ) {
        let mut guard = self.sink.write().await;
        *guard = Some(Arc::from(sink));
    }

    async fn on_unregister(&self) {
        let mut guard = self.sink.write().await;
        *guard = None;
    }

    async fn forward(
        &self,
        _queue: Option<u32>,
        _cla_addr: &ClaAddress,
        _bundle_id: &Id,
        _total_len: u64,
        _stream: &mut (dyn Receiver<Segment> + '_),
    ) -> ClaResult<ForwardBundleResult> {
        // WebBundleInjector is exclusively an ingress sink, not an egress transport
        Ok(ForwardBundleResult::NoNeighbour)
    }
}
