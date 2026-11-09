//! Configuration for the Hamradio BPA node.

pub mod cla;

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

fn default_node_id() -> String {
    "dtn://n0call.ampr.org/".to_string()
}

fn default_grpc_addr() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 0], 50051))
}

/// Root configuration structure for `dtn-hdy-ham`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct HamConfig {
    /// Administrative radio callsign (e.g. `STATA` or `F4JXQ`).
    /// If set and `node_id` is default, `node_id` defaults to `dtn://<callsign>/`.
    #[serde(default)]
    pub callsign: Option<String>,

    /// Administrative Node EID (e.g. `dtn://f4jxq.ampr.org/` or `ipn:1.0`).
    #[serde(default = "default_node_id")]
    pub node_id: String,

    /// Additional administrative EIDs for this node.
    pub admin_endpoints: Vec<String>,

    /// Bundle storage settings (SQLite database).
    pub storage: StorageConfig,

    /// Application service settings (Chat, Files, and storage quota).
    #[serde(default)]
    pub application: ApplicationConfig,

    /// Local gRPC application server settings.
    pub grpc: GrpcConfig,

    /// Web dashboard and REST API server settings (optional).
    pub web: Option<WebConfig>,

    /// Configured Convergence Layer Adapters (TCPCLv4, LTP, HQFBP).
    pub clas: Vec<cla::ClaConfig>,

    /// Static routing definitions.
    pub routes: Vec<RouteConfig>,

    /// SenML telemetry and dashboard configuration.
    #[serde(default)]
    pub senml: Option<SenmlConfig>,

    /// RFC 9171 ingress validation policy toggles.
    #[serde(default)]
    pub rfc9171_validity: Option<Rfc9171ValidityConfig>,

    /// Optional filesystem path this configuration was loaded from.
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}

impl Default for HamConfig {
    fn default() -> Self {
        Self {
            callsign: None,
            node_id: default_node_id(),
            admin_endpoints: Vec::new(),
            storage: StorageConfig::default(),
            application: ApplicationConfig::default(),
            grpc: GrpcConfig::default(),
            web: None,
            clas: Vec::new(),
            routes: Vec::new(),
            senml: None,
            rfc9171_validity: Some(Rfc9171ValidityConfig::default()),
            config_path: None,
        }
    }
}

/// RFC 9171 ingress validation toggles.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Rfc9171ValidityConfig {
    /// Require CRC or BIB on the primary block.
    /// In amateur radio context, defaults to `Some(false)` for out-of-the-box interoperability
    /// with implementations (e.g. dtn-android-messenger / dtn7-rs) that do not generate a primary block CRC.
    pub primary_block_integrity: Option<bool>,

    /// Require a Bundle Age block when the creation time has no clock.
    pub bundle_age_required: Option<bool>,
}

impl Default for Rfc9171ValidityConfig {
    fn default() -> Self {
        Self {
            primary_block_integrity: Some(false),
            bundle_age_required: Some(false),
        }
    }
}

/// Storage options for bundles and metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct StorageConfig {
    /// Directory for pending bundle payloads and SQLite database. Defaults to `~/.local/share/dtn-hdy-ham`.
    pub data_dir: Option<PathBuf>,

    /// Path to SQLite database file. Kept for backwards compatibility.
    pub sqlite_path: Option<PathBuf>,

    /// Run with in-memory volatile storage (useful for testing and ephemeral nodes).
    pub in_memory: bool,
}

impl StorageConfig {
    /// Returns the effective base directory for storage.
    pub fn effective_data_dir(&self) -> PathBuf {
        if let Some(ref dir) = self.data_dir {
            dir.clone()
        } else if let Some(ref db) = self.sqlite_path {
            db.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
        } else if let Some(base_dirs) = directories::BaseDirs::new() {
            base_dirs.data_local_dir().join("dtn-hdy-ham")
        } else {
            PathBuf::from("./data")
        }
    }

    /// Returns the effective SQLite database file path.
    pub fn effective_db_path(&self) -> PathBuf {
        if let Some(ref db) = self.sqlite_path {
            db.clone()
        } else {
            self.effective_data_dir().join("bundles.db")
        }
    }

    /// Returns the effective directory for storing bundle payloads.
    pub fn effective_payload_dir(&self) -> PathBuf {
        self.effective_data_dir().join("payloads")
    }
}

/// Application service settings (e.g. Chat, Files, and storage quota).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ApplicationConfig {
    /// Storage quota policy.
    pub quota: QuotaConfig,
}

/// Application storage quota configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct QuotaConfig {
    /// Maximum disk space allowed for application data (chat DB + files payload dir).
    /// Accepts numbers (e.g. `314572800`) or human-readable strings (e.g. `"300MB"`, `"300MiB"`, `"1GB"`).
    /// Defaults to 300 MB (314,572,800 bytes).
    #[serde(
        deserialize_with = "deserialize_byte_size",
        serialize_with = "serialize_byte_size"
    )]
    pub max_storage: u64,
}

pub const DEFAULT_APP_MAX_STORAGE_BYTES: u64 = 300 * 1024 * 1024; // 300 MB

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            max_storage: DEFAULT_APP_MAX_STORAGE_BYTES,
        }
    }
}

/// Parses human-readable byte sizes (e.g. `"300MB"`, `"300MiB"`, `"1GB"` or numeric `"1024"`).
pub fn parse_byte_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty byte size".to_string());
    }
    if let Ok(n) = s.parse::<u64>() {
        return Ok(n);
    }
    let num_end = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(num_end);
    let val: f64 = num_str
        .trim()
        .parse()
        .map_err(|e| format!("invalid number '{num_str}': {e}"))?;
    let unit = unit_str.trim().to_uppercase();
    let multiplier: f64 = match unit.as_str() {
        "" | "B" | "BYTES" => 1.0,
        "K" | "KB" | "KIB" => 1024.0,
        "M" | "MB" | "MIB" => 1024.0 * 1024.0,
        "G" | "GB" | "GIB" => 1024.0 * 1024.0 * 1024.0,
        "T" | "TB" | "TIB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        other => return Err(format!("unknown byte unit '{other}'")),
    };
    Ok((val * multiplier) as u64)
}

fn deserialize_byte_size<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ByteSizeVisitor;

    impl<'de> serde::de::Visitor<'de> for ByteSizeVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a byte size string (e.g. '300MB', '500KB') or an integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<u64, E>
        where
            E: serde::de::Error,
        {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<u64, E>
        where
            E: serde::de::Error,
        {
            if value < 0 {
                return Err(E::custom("byte size cannot be negative"));
            }
            Ok(value as u64)
        }

        fn visit_str<E>(self, value: &str) -> Result<u64, E>
        where
            E: serde::de::Error,
        {
            parse_byte_size(value).map_err(E::custom)
        }
    }

    deserializer.deserialize_any(ByteSizeVisitor)
}

fn serialize_byte_size<S>(val: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let bytes = *val;
    let s = if bytes > 0 && bytes.is_multiple_of(1024 * 1024 * 1024) {
        format!("{}GB", bytes / (1024 * 1024 * 1024))
    } else if bytes > 0 && bytes.is_multiple_of(1024 * 1024) {
        format!("{}MB", bytes / (1024 * 1024))
    } else if bytes > 0 && bytes.is_multiple_of(1024) {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    };
    serializer.serialize_str(&s)
}

/// gRPC application interface settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct GrpcConfig {
    /// Enable the gRPC application service (required for `dtnsend`, `dtnprint`, etc.).
    pub enabled: bool,

    /// Local address to bind the gRPC server.
    pub address: SocketAddr,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            address: default_grpc_addr(),
        }
    }
}

fn default_web_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8088))
}

pub fn default_web_static_dir() -> PathBuf {
    if Path::new("web/dist").exists() {
        PathBuf::from("web/dist")
    } else if Path::new("dtn-hdy-ham/web/dist").exists() {
        PathBuf::from("dtn-hdy-ham/web/dist")
    } else if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));
        let candidate = exe_dir.join("web").join("dist");
        if candidate.exists() {
            return candidate;
        }
        if let Some(parent) = exe_dir.parent() {
            let candidate_parent = parent.join("web").join("dist");
            if candidate_parent.exists() {
                return candidate_parent;
            }
        }
        PathBuf::from("web/dist")
    } else {
        PathBuf::from("web/dist")
    }
}

/// Web dashboard and REST API interface settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WebConfig {
    /// Local address to bind the web server (e.g. `127.0.0.1:8080`).
    #[serde(default = "default_web_addr")]
    pub address: SocketAddr,

    /// Directory containing static web assets (Vite dist).
    #[serde(default = "default_web_static_dir", alias = "web-dist")]
    pub static_dir: PathBuf,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            address: default_web_addr(),
            static_dir: default_web_static_dir(),
        }
    }
}

/// Static routing entry mapping an EID pattern to a CLA and next hop.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RouteConfig {
    /// EID pattern to match (e.g. `dtn://*/*` or `ipn:*.*`).
    pub pattern: String,

    /// Target CLA name.
    pub cla: String,

    /// Optional next hop address (e.g. `44.27.131.233:4556` or callsign).
    pub next_hop: Option<String>,
}

/// SenML telemetry configuration block.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct SenmlConfig {
    /// List of ordered dashboard metrics.
    #[serde(default)]
    pub metrics: Vec<DashboardMetricConfig>,
}

/// A configured telemetry metric card for the SenML dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DashboardMetricConfig {
    /// Full SenML metric name (e.g. `urn:dtn:node1:power/bus_voltage`).
    pub name: String,

    /// Optional human-readable display alias/label (e.g. `Tension Bus`).
    #[serde(default)]
    pub label: Option<String>,
}

impl HamConfig {
    /// Checks if a configuration file exists at explicit path or in standard locations.
    pub fn find_config(path: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = path {
            if p.exists() {
                Some(p.to_path_buf())
            } else {
                None
            }
        } else {
            Self::find_default_config()
        }
    }

    /// Loads configuration from an explicit file path or searches default standard locations.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let candidate_path = if let Some(p) = path {
            if !p.exists() {
                anyhow::bail!("Configuration file not found: {}", p.display());
            }
            Some(p.to_path_buf())
        } else {
            Self::find_default_config()
        };

        if let Some(config_file) = candidate_path {
            tracing::info!("Loading configuration from {}", config_file.display());
            let s = config::Config::builder()
                .add_source(config::File::from(config_file.clone()))
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to read configuration: {e}"))?;
            let mut cfg = s
                .try_deserialize::<HamConfig>()
                .map_err(|e| anyhow::anyhow!("Failed to parse configuration: {e}"))?;
            cfg.config_path = Some(config_file);
            cfg.normalize();
            Ok(cfg)
        } else {
            tracing::info!("No configuration file found; using default hamradio configuration");
            let mut cfg = Self::default();
            cfg.normalize();
            Ok(cfg)
        }
    }

    /// Normalizes configuration defaults, such as deriving node_id from callsign
    /// and propagating root callsign to CLAs.
    pub fn normalize(&mut self) {
        if let Some(ref mut d) = self.storage.data_dir {
            *d = crate::wizard::expand_tilde(d);
        }
        if let Some(ref mut p) = self.storage.sqlite_path {
            *p = crate::wizard::expand_tilde(p);
        }
        if let Some(ref cs) = self.callsign {
            if self.node_id == default_node_id() {
                self.node_id = format!("dtn://{}/", cs.to_lowercase());
            }
            // Propagate callsign to CLAs that require it if missing
            for cla in &mut self.clas {
                if let cla::ClaType::MercuryArq(ref mut m_cfg) = cla.cla_type
                    && m_cfg.my_callsign.is_none()
                {
                    m_cfg.my_callsign = Some(cs.clone());
                }
            }
        }
    }

    fn find_default_config() -> Option<PathBuf> {
        let local_names = ["dtn-hdy-ham.yaml", "dtn-hdy-ham.toml", "config.yaml"];
        for name in local_names {
            let p = PathBuf::from(name);
            if p.exists() {
                return Some(p);
            }
        }

        if let Some(base_dirs) = directories::BaseDirs::new() {
            let user_conf = base_dirs
                .config_dir()
                .join("dtn-hdy-ham")
                .join("config.yaml");
            if user_conf.exists() {
                return Some(user_conf);
            }
        }

        let sys_conf = PathBuf::from("/etc/dtn-hdy-ham/config.yaml");
        if sys_conf.exists() {
            return Some(sys_conf);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_byte_size() {
        assert_eq!(parse_byte_size("1024").unwrap(), 1024);
        assert_eq!(parse_byte_size("1024B").unwrap(), 1024);
        assert_eq!(parse_byte_size("1KB").unwrap(), 1024);
        assert_eq!(parse_byte_size("1KiB").unwrap(), 1024);
        assert_eq!(parse_byte_size("300MB").unwrap(), 300 * 1024 * 1024);
        assert_eq!(parse_byte_size("300MiB").unwrap(), 300 * 1024 * 1024);
        assert_eq!(
            parse_byte_size("1.5GB").unwrap(),
            (1.5 * 1024.0 * 1024.0 * 1024.0) as u64
        );
        assert_eq!(parse_byte_size(" 500 MB ").unwrap(), 500 * 1024 * 1024);
        assert!(parse_byte_size("").is_err());
        assert!(parse_byte_size("invalid").is_err());
        assert!(parse_byte_size("100XYZ").is_err());
    }

    #[test]
    fn test_serde_application_quota_config() {
        // 1. With human-readable string "500MB"
        let yaml_str = r#"
application:
  quota:
    max-storage: "500MB"
"#;
        let cfg: HamConfig = serde_yaml::from_str(yaml_str).unwrap();
        assert_eq!(cfg.application.quota.max_storage, 500 * 1024 * 1024);

        // 2. With numeric value
        let yaml_num = r#"
application:
  quota:
    max-storage: 10485760
"#;
        let cfg_num: HamConfig = serde_yaml::from_str(yaml_num).unwrap();
        assert_eq!(cfg_num.application.quota.max_storage, 10485760);

        // 3. Omitted (default 300MB)
        let yaml_default = "node-id: 'dtn://test/'";
        let cfg_default: HamConfig = serde_yaml::from_str(yaml_default).unwrap();
        assert_eq!(
            cfg_default.application.quota.max_storage,
            DEFAULT_APP_MAX_STORAGE_BYTES
        );
    }
}
