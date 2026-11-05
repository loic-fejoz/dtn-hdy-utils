//! CLA manager for runtime CLA interaction, peer status tracking and connection management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Connection status of a remote CLA peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerConnectionStatus {
    Connected,
    Disconnected,
    Connecting,
}

/// Status details for a registered CLA peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatusInfo {
    pub address: String,
    pub status: PeerConnectionStatus,
    pub last_attempt_at: Option<String>,
}

fn now_iso() -> String {
    let now = OffsetDateTime::now_utc();
    now.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| now.to_string())
}

type RegisteredTcpPeer = (Arc<hardy_tcpclv4::Tcpclv4>, SocketAddr);

/// Thread-safe manager for configured CLAs and their peers.
#[derive(Clone, Default)]
pub struct ClaManager {
    tcp_peers: Arc<RwLock<Vec<RegisteredTcpPeer>>>,
    statuses: Arc<RwLock<HashMap<SocketAddr, PeerStatusInfo>>>,
}

impl ClaManager {
    /// Creates a new empty CLA manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tcp_peers: Arc::new(RwLock::new(Vec::new())),
            statuses: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers a TCPCLv4 instance with a remote peer socket address.
    pub async fn register_tcp_peer(&self, cla: Arc<hardy_tcpclv4::Tcpclv4>, addr: SocketAddr) {
        let mut guard = self.tcp_peers.write().await;
        guard.push((cla, addr));

        let mut st = self.statuses.write().await;
        st.entry(addr).or_insert_with(|| PeerStatusInfo {
            address: addr.to_string(),
            status: PeerConnectionStatus::Disconnected,
            last_attempt_at: None,
        });
    }

    /// Updates the connection status for a peer.
    pub async fn set_peer_status(&self, addr: SocketAddr, status: PeerConnectionStatus) {
        let mut st = self.statuses.write().await;
        st.insert(
            addr,
            PeerStatusInfo {
                address: addr.to_string(),
                status,
                last_attempt_at: Some(now_iso()),
            },
        );
    }

    /// Checks if a peer is currently marked as connected.
    pub async fn is_peer_connected(&self, addr: &SocketAddr) -> bool {
        let st = self.statuses.read().await;
        st.get(addr)
            .map(|info| info.status == PeerConnectionStatus::Connected)
            .unwrap_or(false)
    }

    /// Returns a list of all tracked peer statuses.
    pub async fn list_peer_statuses(&self) -> Vec<PeerStatusInfo> {
        let st = self.statuses.read().await;
        st.values().cloned().collect()
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
                info!("Connection attempt initiated for TCPCL peer {}", addr);
                self.set_peer_status(*addr, PeerConnectionStatus::Connecting)
                    .await;

                match cla.connect(addr).await {
                    Ok(()) => {
                        info!("TCPCL connection to {} established successfully", addr);
                        self.set_peer_status(*addr, PeerConnectionStatus::Connected)
                            .await;
                        return Ok(());
                    }
                    Err(e) => {
                        warn!("TCPCL connection to {} failed: {}", addr, e);
                        self.set_peer_status(*addr, PeerConnectionStatus::Disconnected)
                            .await;
                        anyhow::bail!("TCPCL connection to {} failed: {e}", addr);
                    }
                }
            }
        }

        anyhow::bail!(
            "No configured TCPCL adapter found for peer address '{}'",
            target_str
        )
    }
}
