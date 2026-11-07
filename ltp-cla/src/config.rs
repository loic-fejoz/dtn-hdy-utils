//! Configuration for the LTP Convergence Layer Adapter.

use std::net::SocketAddr;
use std::time::Duration;
use std::vec::Vec;

use hardy_bpv7::eid::NodeId;
use hardy_ltp::types::EngineId;

fn default_udp_bind() -> SocketAddr {
    "0.0.0.0:1113".parse().expect("valid socket addr")
}

const fn default_mtu() -> usize {
    1400
}

const fn default_rtt() -> Duration {
    Duration::from_millis(500)
}

const fn default_ack_margin() -> Duration {
    Duration::from_millis(250)
}

const fn default_rtx_limit() -> u32 {
    10
}

const fn default_reconnect_initial() -> Duration {
    Duration::from_millis(500)
}

const fn default_reconnect_max() -> Duration {
    Duration::from_secs(30)
}

const fn default_burst_capacity() -> usize {
    2048
}

const fn default_turnaround_timeout() -> Duration {
    Duration::from_secs(3)
}

const fn default_dwell_time() -> Duration {
    Duration::from_millis(500)
}

/// Turnaround pacing mode when operating over a half-duplex (simplex) radio channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum TurnaroundMode {
    /// Wait for incoming Report Segment (RS) from peer, with a fallback timeout (default).
    #[default]
    WaitForReport,
    /// Observe a fixed quiet duration (dwell time) after each Checkpoint before resuming.
    FixedDwell,
}

/// A configured remote peer mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub struct PeerEntry {
    /// Remote LTP Engine ID.
    pub engine_id: EngineId,
    /// Destination socket address for UDP or KISS connection.
    #[cfg_attr(feature = "serde", serde(default))]
    pub address: Option<SocketAddr>,
    /// Associated BPA Node ID.
    #[cfg_attr(feature = "serde", serde(default))]
    pub node_id: Option<NodeId>,
    /// Estimated nominal round-trip time for this peer (e.g. GEO ~500ms, cislunar ~3s).
    #[cfg_attr(feature = "serde", serde(default))]
    pub rtt: Option<Duration>,
    /// Extra margin added to retransmission timers for this peer.
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack_margin: Option<Duration>,
}

impl Default for PeerEntry {
    fn default() -> Self {
        Self {
            engine_id: EngineId(0),
            address: None,
            node_id: None,
            rtt: None,
            ack_margin: None,
        }
    }
}

/// The underlying link transport utilized by the LTP CLA.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "kebab-case"))]
pub enum TransportMode {
    /// Standard UDP datagram transport (RFC 7122).
    #[cfg_attr(feature = "serde", serde(rename = "udp", rename_all = "kebab-case"))]
    Udp {
        /// Local socket address to bind and listen on (e.g. `0.0.0.0:1113`).
        #[cfg_attr(feature = "serde", serde(default = "default_udp_bind"))]
        bind_addr: SocketAddr,
    },
    /// KISS over TCP framing for packet radio / TNC (e.g. Direwolf).
    #[cfg_attr(
        feature = "serde",
        serde(rename = "kiss-tcp", rename_all = "kebab-case")
    )]
    KissTcp {
        /// Remote KISS TCP server socket address (e.g. `127.0.0.1:8001`).
        server_addr: SocketAddr,
        /// Optional local address to bind when connecting out.
        #[cfg_attr(feature = "serde", serde(default))]
        bind_addr: Option<SocketAddr>,
        /// KISS port nibble (0..15, default 0).
        #[cfg_attr(feature = "serde", serde(default))]
        port: u8,
        /// Initial reconnect delay on connection loss (default 500ms).
        #[cfg_attr(
            feature = "serde",
            serde(default = "default_reconnect_initial", with = "humantime_duration")
        )]
        reconnect_initial_delay: Duration,
        /// Maximum reconnect delay with exponential backoff (default 30s).
        #[cfg_attr(
            feature = "serde",
            serde(default = "default_reconnect_max", with = "humantime_duration")
        )]
        reconnect_max_delay: Duration,
        /// Minimum frame length in bytes required by the underlying TNC (e.g. 15 for Direwolf).
        /// Defaults to 0 (no padding, preserving minimal frame size for modems like Mercury).
        #[cfg_attr(feature = "serde", serde(default))]
        min_frame_len: usize,
        /// Optional rate limit in bytes per second for the aggregate KISS stream (e.g. 1200 or 9600).
        /// If `None`, transmission is unthrottled.
        #[cfg_attr(feature = "serde", serde(default))]
        rate_limit_bytes_per_sec: Option<u64>,
        /// Maximum burst capacity in bytes for the leaky bucket. Defaults to 2048 bytes if rate limiting is enabled.
        #[cfg_attr(feature = "serde", serde(default = "default_burst_capacity"))]
        burst_capacity_bytes: usize,
        /// Enables half-duplex (simplex) channel pacing. When enabled, emitting a Checkpoint pauses the port.
        #[cfg_attr(feature = "serde", serde(default))]
        half_duplex: bool,
        /// Turnaround mode when operating in half-duplex. Defaults to `WaitForReport`.
        #[cfg_attr(feature = "serde", serde(default))]
        turnaround_mode: TurnaroundMode,
        /// Maximum timeout to wait for Report Segment before unblocking in `WaitForReport` mode (default: 3s).
        #[cfg_attr(
            feature = "serde",
            serde(default = "default_turnaround_timeout", with = "humantime_duration")
        )]
        turnaround_timeout: Duration,
        /// Fixed quiet / dwell duration after each Checkpoint in `FixedDwell` mode (default: 500ms).
        #[cfg_attr(
            feature = "serde",
            serde(default = "default_dwell_time", with = "humantime_duration")
        )]
        dwell_time: Duration,
    },
}

impl TransportMode {
    /// Helper to create a KISS TCP transport mode with standard reconnect defaults.
    #[must_use]
    pub fn kiss_tcp(server_addr: SocketAddr, port: u8) -> Self {
        Self::KissTcp {
            server_addr,
            bind_addr: None,
            port,
            reconnect_initial_delay: default_reconnect_initial(),
            reconnect_max_delay: default_reconnect_max(),
            min_frame_len: 0,
            rate_limit_bytes_per_sec: None,
            burst_capacity_bytes: default_burst_capacity(),
            half_duplex: false,
            turnaround_mode: TurnaroundMode::default(),
            turnaround_timeout: default_turnaround_timeout(),
            dwell_time: default_dwell_time(),
        }
    }

    /// Helper to create a KISS TCP transport mode with custom reconnect parameters.
    #[must_use]
    pub fn kiss_tcp_with_reconnect(
        server_addr: SocketAddr,
        port: u8,
        reconnect_initial_delay: Duration,
        reconnect_max_delay: Duration,
    ) -> Self {
        Self::KissTcp {
            server_addr,
            bind_addr: None,
            port,
            reconnect_initial_delay,
            reconnect_max_delay,
            min_frame_len: 0,
            rate_limit_bytes_per_sec: None,
            burst_capacity_bytes: default_burst_capacity(),
            half_duplex: false,
            turnaround_mode: TurnaroundMode::default(),
            turnaround_timeout: default_turnaround_timeout(),
            dwell_time: default_dwell_time(),
        }
    }

    /// Sets the minimum frame length in bytes for TNC compatibility.
    #[must_use]
    pub fn with_min_frame_len(mut self, min_len: usize) -> Self {
        if let Self::KissTcp {
            min_frame_len: ref mut len,
            ..
        } = self
        {
            *len = min_len;
        }
        self
    }

    /// Configures the leaky-bucket rate limiter.
    #[must_use]
    pub fn with_rate_limit(mut self, rate_bytes_per_sec: u64, burst_capacity: usize) -> Self {
        if let Self::KissTcp {
            rate_limit_bytes_per_sec: ref mut r,
            burst_capacity_bytes: ref mut b,
            ..
        } = self
        {
            *r = Some(rate_bytes_per_sec);
            *b = burst_capacity;
        }
        self
    }

    /// Configures half-duplex pacing with turnaround timeout.
    #[must_use]
    pub fn with_half_duplex(mut self, mode: TurnaroundMode, timeout: Duration) -> Self {
        if let Self::KissTcp {
            half_duplex: ref mut hd,
            turnaround_mode: ref mut tm,
            turnaround_timeout: ref mut to,
            ..
        } = self
        {
            *hd = true;
            *tm = mode;
            *to = timeout;
        }
        self
    }

    /// Configures fixed quiet / dwell time after Checkpoint.
    #[must_use]
    pub fn with_dwell_time(mut self, dwell: Duration) -> Self {
        if let Self::KissTcp {
            half_duplex: ref mut hd,
            turnaround_mode: ref mut tm,
            dwell_time: ref mut dt,
            ..
        } = self
        {
            *hd = true;
            *tm = TurnaroundMode::FixedDwell;
            *dt = dwell;
        }
        self
    }
}

/// Configuration settings for the LTP CLA.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, rename_all = "kebab-case"))]
pub struct Config {
    /// Local LTP Engine ID.
    pub engine_id: EngineId,
    /// Link transport mode.
    pub transport: TransportMode,
    /// Maximum segment payload length in bytes (default 1400).
    #[cfg_attr(feature = "serde", serde(default = "default_mtu"))]
    pub max_segment_payload_len: usize,
    /// Estimated Round-Trip Time.
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_rtt", with = "humantime_duration")
    )]
    pub rtt: Duration,
    /// Remote acknowledgement processing margin.
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_ack_margin", with = "humantime_duration")
    )]
    pub ack_margin: Duration,
    /// Checkpoint retransmission limit before cancellation.
    #[cfg_attr(feature = "serde", serde(default = "default_rtx_limit"))]
    pub checkpoint_rtx_limit: u32,
    /// Report retransmission limit before cancellation.
    #[cfg_attr(feature = "serde", serde(default = "default_rtx_limit"))]
    pub report_rtx_limit: u32,
    /// Pre-configured peer entries.
    #[cfg_attr(feature = "serde", serde(default))]
    pub peers: Vec<PeerEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine_id: EngineId(1),
            transport: TransportMode::Udp {
                bind_addr: default_udp_bind(),
            },
            max_segment_payload_len: default_mtu(),
            rtt: default_rtt(),
            ack_margin: default_ack_margin(),
            checkpoint_rtx_limit: default_rtx_limit(),
            report_rtx_limit: default_rtx_limit(),
            peers: Vec::new(),
        }
    }
}

#[cfg(feature = "serde")]
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

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    #[test]
    fn test_serde_kiss_tcp_pacing_config() {
        let yaml_str = r#"
type: kiss-tcp
server-addr: "127.0.0.1:8100"
port: 1
min-frame-len: 15
rate-limit-bytes-per-sec: 1200
burst-capacity-bytes: 1024
half-duplex: true
turnaround-mode: fixed-dwell
turnaround-timeout: 4s
dwell-time: 750ms
"#;
        let transport: TransportMode = serde_yaml::from_str(yaml_str).unwrap();
        match transport {
            TransportMode::KissTcp {
                server_addr,
                port,
                min_frame_len,
                rate_limit_bytes_per_sec,
                burst_capacity_bytes,
                half_duplex,
                turnaround_mode,
                turnaround_timeout,
                dwell_time,
                ..
            } => {
                assert_eq!(server_addr, "127.0.0.1:8100".parse().unwrap());
                assert_eq!(port, 1);
                assert_eq!(min_frame_len, 15);
                assert_eq!(rate_limit_bytes_per_sec, Some(1200));
                assert_eq!(burst_capacity_bytes, 1024);
                assert!(half_duplex);
                assert_eq!(turnaround_mode, TurnaroundMode::FixedDwell);
                assert_eq!(turnaround_timeout, Duration::from_secs(4));
                assert_eq!(dwell_time, Duration::from_millis(750));
            }
            _ => panic!("Expected KissTcp"),
        }
    }
}
