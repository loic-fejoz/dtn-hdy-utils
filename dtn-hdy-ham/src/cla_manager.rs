//! CLA manager for runtime CLA interaction and manual peer connection.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Thread-safe manager for configured CLAs and their peers.
#[derive(Clone, Default)]
pub struct ClaManager {
    tcp_peers: Arc<RwLock<Vec<(Arc<hardy_tcpclv4::Tcpclv4>, SocketAddr)>>>,
}

impl ClaManager {
    /// Creates a new empty CLA manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tcp_peers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Registers a TCPCLv4 instance with a remote peer socket address.
    pub async fn register_tcp_peer(&self, cla: Arc<hardy_tcpclv4::Tcpclv4>, addr: SocketAddr) {
        let mut guard = self.tcp_peers.write().await;
        guard.push((cla, addr));
    }

    /// Initiates an active connection attempt to a specified peer address.
    pub async fn connect_peer(&self, peer_str: &str) -> anyhow::Result<()> {
        let target_str = peer_str.trim();
        let guard = self.tcp_peers.read().await;

        for (cla, addr) in guard.iter() {
            let addr_str = addr.to_string();
            let matches = addr_str == target_str
                || target_str
                    .parse::<SocketAddr>()
                    .map(|a| a == *addr)
                    .unwrap_or(false);

            if matches {
                info!("Manual connection triggered for TCPCL peer {}", addr);
                if let Err(e) = cla.connect(addr).await {
                    tracing::warn!("Manual connection to TCPCL peer {} failed: {}", addr, e);
                    anyhow::bail!("TCPCL connection to {} failed: {e}", addr);
                }
                info!("Manual connection to TCPCL peer {} succeeded", addr);
                return Ok(());
            }
        }

        anyhow::bail!(
            "No configured TCPCL adapter found for peer address '{}'",
            target_str
        )
    }
}
