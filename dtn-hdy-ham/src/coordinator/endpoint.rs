//! Abstraction for CLA contact execution endpoints.

use hardy_bpa::async_trait;
use hardy_bpv7::eid::NodeId;

/// An execution endpoint that can initiate and terminate contacts on command from the [`ContactCoordinator`].
#[async_trait]
#[allow(clippy::double_must_use)]
pub trait ContactExecutionEndpoint: Send + Sync {
    /// Initiate a contact with the given remote peer callsign and optional EID.
    async fn start_contact(&self, callsign: &str, node_id: Option<&NodeId>) -> Result<(), String>;

    /// Terminate an active contact with the peer.
    async fn end_contact(&self, callsign: &str) -> Result<(), String>;

    /// Check if a contact is currently active.
    async fn is_active(&self) -> bool;
}

#[async_trait]
impl ContactExecutionEndpoint for hardy_mercury_cla::MercuryCla {
    async fn start_contact(&self, callsign: &str, node_id: Option<&NodeId>) -> Result<(), String> {
        self.connect_peer(callsign, node_id.cloned())
            .await
            .map_err(|e| e.to_string())
    }

    async fn end_contact(&self, _callsign: &str) -> Result<(), String> {
        self.disconnect().await.map_err(|e| e.to_string())
    }

    async fn is_active(&self) -> bool {
        self.is_connected().await
    }
}
