//! Convergence Layer Adapter configuration for Hamradio BPA.

use core::num::NonZeroU64;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// CLA entry in configuration file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaConfig {
    /// Human-readable unique name for this CLA instance.
    pub name: String,
    /// Specific CLA type and configuration.
    #[serde(flatten)]
    pub cla_type: ClaType,
    /// Optional egress policy name.
    #[serde(default)]
    pub policy: Option<String>,
}

/// Supported CLA protocol engines.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClaType {
    /// TCPCL v4 convergence layer (RFC 9174).
    #[serde(rename = "tcpclv4", alias = "tcpcl")]
    TcpClv4(Tcpclv4HamConfig),

    /// Licklider Transmission Protocol (LTP, RFC 5326 / RFC 7122 / KISS).
    #[serde(rename = "ltp")]
    Ltp(hardy_ltp_cla::Config),

    /// Hamradio Quick File Broadcasting Protocol (HQFBP / KISS TCP / Serial).
    #[serde(rename = "hqfbp")]
    Hqfbp(hardy_hqfbp_cla::Config),

    /// Mercury connected ARQ convergence layer.
    #[serde(rename = "mercury-arq", alias = "mercury")]
    MercuryArq(MercuryArqHamConfig),

    /// Fallback for unrecognized CLA types.
    #[serde(untagged)]
    Other {
        #[serde(rename = "type")]
        cla_type: String,
        #[serde(flatten)]
        config: serde_json::Value,
    },
}

/// Simplified Hamradio-friendly configuration for TCPCLv4.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Tcpclv4HamConfig {
    /// Single listening socket address (e.g. `"[::]:4556"`).
    pub address: Option<SocketAddr>,

    /// Multiple listening addresses.
    pub listeners: Option<Vec<SocketAddr>>,

    /// Whether TLS is mandatory. Disabled by default for amateur radio compliance (Hamnet / 44Net).
    #[serde(alias = "require_tls", alias = "require-tls")]
    pub require_tls: Option<bool>,

    /// Outbound peer socket addresses to connect to (e.g. `["44.27.131.233:4556"]`).
    pub peers: Option<Vec<String>>,

    /// Largest acceptable single-segment payload in bytes.
    pub segment_mru: Option<NonZeroU64>,

    /// Largest acceptable total bundle transfer in bytes.
    pub transfer_mru: Option<NonZeroU64>,
}

fn default_mercury_control() -> SocketAddr {
    "127.0.0.1:8300".parse().expect("valid address")
}

fn default_mercury_data() -> SocketAddr {
    "127.0.0.1:8301".parse().expect("valid address")
}

fn default_bandwidth() -> u32 {
    2300
}

/// Configuration for Mercury Connected ARQ CLA.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MercuryArqHamConfig {
    /// Local amateur radio callsign (inherits from root config if not set).
    pub my_callsign: Option<String>,
    /// Control TCP port address (default: 127.0.0.1:8300).
    pub control_addr: SocketAddr,
    /// Data TCP port address (default: 127.0.0.1:8301).
    pub data_addr: SocketAddr,
    /// ARQ bandwidth in Hz (default: 2300).
    pub bandwidth: u32,
    /// Idle disconnect timeout.
    #[serde(with = "humantime_duration_opt", default)]
    pub idle_timeout: Option<std::time::Duration>,
    /// Grace period for in-flight bundle drain upon window expiration (default: 30s).
    #[serde(with = "humantime_duration_opt", default)]
    pub grace_period: Option<std::time::Duration>,
    /// Connect handshake timeout (default: 30s).
    #[serde(with = "humantime_duration_opt", default)]
    pub connect_timeout: Option<std::time::Duration>,
    /// Configured peers and contact schedules.
    pub peers: Vec<MercuryPeerHamConfig>,
}

impl Default for MercuryArqHamConfig {
    fn default() -> Self {
        Self {
            my_callsign: None,
            control_addr: default_mercury_control(),
            data_addr: default_mercury_data(),
            bandwidth: default_bandwidth(),
            idle_timeout: None,
            grace_period: Some(std::time::Duration::from_secs(30)),
            connect_timeout: Some(std::time::Duration::from_secs(30)),
            peers: Vec::new(),
        }
    }
}

/// Configuration entry for a peer in Mercury Connected ARQ mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MercuryPeerHamConfig {
    /// Remote callsign.
    pub callsign: String,
    /// Remote node EID. If omitted, defaults to `dtn://<callsign>/`.
    #[serde(default)]
    pub node_id: Option<String>,
    /// Optional scheduled contact plan.
    #[serde(default)]
    pub schedule: Option<ScheduleConfig>,
}

impl MercuryPeerHamConfig {
    /// Returns the effective Node ID string for this peer.
    #[must_use]
    pub fn effective_node_id(&self) -> String {
        if let Some(ref nid) = self.node_id {
            nid.clone()
        } else {
            format!("dtn://{}/", self.callsign.to_lowercase())
        }
    }
}

/// Contact schedule configuration (recurring cron or one-shot).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum ScheduleConfig {
    #[serde(rename = "recurring", alias = "cron")]
    Recurring {
        cron: String,
        #[serde(with = "humantime_duration")]
        duration: std::time::Duration,
    },
    #[serde(rename = "one-shot", alias = "oneshot")]
    OneShot {
        #[serde(default)]
        start: Option<String>,
        #[serde(default)]
        end: Option<String>,
    },
}

mod humantime_duration {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&humantime::format_duration(*duration))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let text = String::deserialize(d)?;
        humantime::parse_duration(&text).map_err(serde::de::Error::custom)
    }
}

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
