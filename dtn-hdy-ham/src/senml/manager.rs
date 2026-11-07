//! Central SenML telemetry manager coordinating storage and live SSE distribution.

use tokio::sync::broadcast;

use super::storage::{SenmlStorage, TelemetryPoint};

/// Shared handle for managing SenML telemetry data and real-time streaming.
#[derive(Clone)]
pub struct SenmlManager {
    storage: SenmlStorage,
    broadcast_tx: broadcast::Sender<TelemetryPoint>,
}

impl SenmlManager {
    /// Creates a new `SenmlManager` wrapping the underlying storage and initializing broadcast bus.
    pub fn new(storage: SenmlStorage) -> Self {
        let (broadcast_tx, _) = broadcast::channel(512);
        Self {
            storage,
            broadcast_tx,
        }
    }

    /// Accessor for the underlying SQLite storage.
    pub fn storage(&self) -> &SenmlStorage {
        &self.storage
    }

    /// Broadcasts newly ingested telemetry points to connected SSE subscribers.
    pub fn broadcast_points(&self, points: Vec<TelemetryPoint>) {
        for pt in points {
            let _ = self.broadcast_tx.send(pt);
        }
    }

    /// Subscribes to the real-time telemetry broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<TelemetryPoint> {
        self.broadcast_tx.subscribe()
    }
}
