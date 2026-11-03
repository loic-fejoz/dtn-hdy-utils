use std::net::SocketAddr;
use std::time::Duration;

use hardy_bpv7::eid::NodeId;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

fn default_control_addr() -> SocketAddr {
    "127.0.0.1:8300".parse().expect("valid address")
}

fn default_data_addr() -> SocketAddr {
    "127.0.0.1:8301".parse().expect("valid address")
}

fn default_bandwidth() -> u32 {
    2300
}

fn default_connect_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_grace_period() -> Duration {
    Duration::from_secs(30)
}

/// Configuration entry for a known Mercury peer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "kebab-case")
)]
pub struct PeerEntry {
    /// Amateur radio callsign of the remote station.
    pub callsign: String,

    /// Node EID of the remote station. If omitted, defaults to `dtn://<callsign>/`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub node_id: Option<NodeId>,
}

impl PeerEntry {
    /// Returns the effective NodeId for this peer.
    #[must_use]
    pub fn effective_node_id(&self) -> NodeId {
        if let Some(ref nid) = self.node_id {
            nid.clone()
        } else {
            let uri = format!("dtn://{}/", self.callsign.to_lowercase());
            uri.parse().expect("valid node EID")
        }
    }
}

/// Configuration for the Mercury Connected ARQ CLA.
#[derive(Debug, Clone)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, rename_all = "kebab-case")
)]
pub struct Config {
    /// Local amateur radio callsign.
    pub my_callsign: String,

    /// Mercury TNC control TCP port address (default: 127.0.0.1:8300).
    pub control_addr: SocketAddr,

    /// Mercury TNC ARQ data TCP port address (default: 127.0.0.1:8301).
    pub data_addr: SocketAddr,

    /// ARQ bandwidth in Hz (e.g. 500, 2300, 2750; default 2300).
    pub bandwidth: u32,

    /// Connect handshake timeout (default: 30s).
    #[cfg_attr(feature = "serde", serde(with = "humantime_duration_opt", default))]
    pub connect_timeout: Option<Duration>,

    /// Grace period after contact duration expires to drain an in-flight bundle (default: 30s).
    #[cfg_attr(feature = "serde", serde(with = "humantime_duration_opt", default))]
    pub grace_period: Option<Duration>,

    /// Idle timeout after which an active session with no traffic is closed (optional).
    #[cfg_attr(feature = "serde", serde(with = "humantime_duration_opt", default))]
    pub idle_timeout: Option<Duration>,

    /// Static or scheduled peers.
    pub peers: Vec<PeerEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            my_callsign: "N0CALL".to_string(),
            control_addr: default_control_addr(),
            data_addr: default_data_addr(),
            bandwidth: default_bandwidth(),
            connect_timeout: Some(default_connect_timeout()),
            grace_period: Some(default_grace_period()),
            idle_timeout: None,
            peers: Vec::new(),
        }
    }
}

#[cfg(feature = "serde")]
mod humantime_duration_opt {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(duration: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match duration {
            Some(d) => s.collect_str(&humantime::format_duration(*d)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt: Option<String> = Option::deserialize(d)?;
        match opt {
            Some(text) => humantime::parse_duration(&text)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}
