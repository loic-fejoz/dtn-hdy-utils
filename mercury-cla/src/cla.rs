//! BPA Convergence Layer Adapter implementation for Mercury connected ARQ mode.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{debug, info};

use hardy_async::TaskPool;
use hardy_bpa::async_trait;
use hardy_bpa::cla::{
    Cla, ClaAddress, ClaInit, ForwardBundleResult, Result as ClaResult, Segment, Sink,
};
use hardy_bpa::stream::{Receiver, buffer_stream};
use hardy_bpv7::bundle::Id;
use hardy_bpv7::eid::NodeId;

use crate::config::{Config, PeerEntry};
use crate::driver::{Driver, DriverCommand};
use crate::error::{Error, Result};

struct ClaInner {
    sink: Arc<dyn Sink>,
    tasks: Arc<TaskPool>,
    cmd_tx: mpsc::Sender<DriverCommand>,
    _driver_handle: tokio::task::JoinHandle<()>,
}

/// The Mercury Connected ARQ Convergence Layer Adapter.
#[derive(Clone)]
pub struct MercuryCla {
    config: Config,
    inner: Arc<RwLock<Option<ClaInner>>>,
}

impl MercuryCla {
    /// Creates a new Mercury CLA instance with the given configuration.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            inner: Arc::new(RwLock::new(None)),
        }
    }

    /// Returns the initialisation parameters for registering this CLA with the BPA.
    #[must_use]
    pub fn cla_init(&self) -> ClaInit {
        ClaInit {
            address_type: None,
            lane_count: NonZeroU32::new(1),
            max_bundle_size: None,
        }
    }

    /// Proactively initiates an ARQ connection to the remote station.
    pub async fn connect_peer(&self, callsign: &str, node_id: Option<NodeId>) -> Result<()> {
        let (cmd_tx, node_id) = {
            let inner_guard = self.inner.read().await;
            let inner = inner_guard.as_ref().ok_or(Error::NotRegistered)?;
            let effective_node_id = if let Some(nid) = node_id {
                nid
            } else {
                let uri = format!("dtn://{}/", callsign.to_lowercase());
                uri.parse().expect("valid node EID")
            };
            (inner.cmd_tx.clone(), effective_node_id)
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(DriverCommand::Connect {
                callsign: callsign.to_string(),
                node_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::DriverClosed)?;

        let timeout = self
            .config
            .connect_timeout
            .unwrap_or(std::time::Duration::from_secs(30));
        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(Error::DriverClosed),
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Disconnects any active ARQ session.
    pub async fn disconnect(&self) -> Result<()> {
        let cmd_tx = {
            let inner_guard = self.inner.read().await;
            let inner = inner_guard.as_ref().ok_or(Error::NotRegistered)?;
            inner.cmd_tx.clone()
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(DriverCommand::Disconnect { reply: reply_tx })
            .await
            .map_err(|_| Error::DriverClosed)?;

        match reply_rx.await {
            Ok(res) => res,
            Err(_) => Err(Error::DriverClosed),
        }
    }

    /// Checks if the CLA currently has an active ARQ session.
    pub async fn is_connected(&self) -> bool {
        let cmd_tx = {
            let inner_guard = self.inner.read().await;
            if let Some(inner) = inner_guard.as_ref() {
                inner.cmd_tx.clone()
            } else {
                return false;
            }
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        if cmd_tx
            .send(DriverCommand::IsConnected { reply: reply_tx })
            .await
            .is_err()
        {
            return false;
        }
        reply_rx.await.unwrap_or(false)
    }

    /// Registers a peer mapping dynamically.
    pub async fn register_peer(&self, peer: PeerEntry) -> Result<()> {
        let cmd_tx = {
            let inner_guard = self.inner.read().await;
            let inner = inner_guard.as_ref().ok_or(Error::NotRegistered)?;
            inner.cmd_tx.clone()
        };

        cmd_tx
            .send(DriverCommand::RegisterPeer { peer })
            .await
            .map_err(|_| Error::DriverClosed)
    }

    /// Unregisters this CLA from the BPA and shuts down the background driver.
    pub async fn unregister(&self) {
        let mut inner_guard = self.inner.write().await;
        if let Some(inner) = inner_guard.take() {
            inner.sink.unregister().await;
            let _ = inner.cmd_tx.send(DriverCommand::Shutdown).await;
            let _ = inner._driver_handle.await;
            inner.tasks.shutdown().await;
        }
    }
}

#[async_trait]
impl Cla for MercuryCla {
    async fn on_register(
        &self,
        sink: Box<dyn Sink>,
        _node_ids: &[NodeId],
        _max_bundle_size: Option<NonZeroU64>,
    ) {
        info!("registering Mercury ARQ CLA with BPA");
        let sink: Arc<dyn Sink> = sink.into();
        let tasks = Arc::new(TaskPool::new());
        let (cmd_tx, cmd_rx) = mpsc::channel(64);

        let driver_handle = Driver::spawn(self.config.clone(), sink.clone(), tasks.clone(), cmd_rx);

        let mut inner_guard = self.inner.write().await;
        *inner_guard = Some(ClaInner {
            sink,
            tasks,
            cmd_tx,
            _driver_handle: driver_handle,
        });
    }

    async fn on_unregister(&self) {
        info!("unregistering Mercury ARQ CLA from BPA");
        let mut inner_guard = self.inner.write().await;
        if let Some(inner) = inner_guard.take() {
            let _ = inner.cmd_tx.send(DriverCommand::Shutdown).await;
            let _ = inner._driver_handle.await;
            inner.tasks.shutdown().await;
        }
    }

    async fn forward(
        &self,
        _lane: Option<u32>,
        _cla_addr: &ClaAddress,
        bundle_id: &Id,
        total_len: u64,
        stream: &mut dyn Receiver<Segment>,
    ) -> ClaResult<ForwardBundleResult> {
        let cmd_tx = {
            let inner_guard = self.inner.read().await;
            let inner = inner_guard
                .as_ref()
                .ok_or(hardy_bpa::cla::Error::Disconnected)?;
            inner.cmd_tx.clone()
        };

        let bundle_bytes = buffer_stream(stream, total_len)
            .await
            .map_err(crate::error::Error::Buffer)?;

        debug!(
            "forwarding bundle {bundle_id} ({} bytes) over Mercury ARQ data socket",
            bundle_bytes.len()
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(DriverCommand::ForwardBundle {
                data: bundle_bytes,
                reply: reply_tx,
            })
            .await
            .map_err(|_| hardy_bpa::cla::Error::Disconnected)?;

        match reply_rx.await {
            Ok(Ok(())) => Ok(ForwardBundleResult::Sent),
            Ok(Err(e)) => Err(hardy_bpa::cla::Error::from(e)),
            Err(_) => Err(hardy_bpa::cla::Error::Disconnected),
        }
    }
}
