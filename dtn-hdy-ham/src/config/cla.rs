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
