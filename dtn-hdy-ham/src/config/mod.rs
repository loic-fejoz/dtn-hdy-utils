//! Configuration for the Hamradio BPA node.

pub mod cla;

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

fn default_node_id() -> String {
    "dtn://n0call.ampr.org/".to_string()
}

fn default_grpc_addr() -> SocketAddr {
    "[::]:50051".parse().expect("valid address")
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

    /// Local gRPC application server settings.
    pub grpc: GrpcConfig,

    /// Web dashboard and REST API server settings (optional).
    pub web: Option<WebConfig>,

    /// Configured Convergence Layer Adapters (TCPCLv4, LTP, HQFBP).
    pub clas: Vec<cla::ClaConfig>,

    /// Static routing definitions.
    pub routes: Vec<RouteConfig>,
}

impl Default for HamConfig {
    fn default() -> Self {
        Self {
            callsign: None,
            node_id: default_node_id(),
            admin_endpoints: Vec::new(),
            storage: StorageConfig::default(),
            grpc: GrpcConfig::default(),
            web: None,
            clas: Vec::new(),
            routes: Vec::new(),
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
    "127.0.0.1:8088".parse().expect("valid address")
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
    #[serde(default = "default_web_static_dir")]
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
                .add_source(config::File::from(config_file))
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to read configuration: {e}"))?;
            let mut cfg = s
                .try_deserialize::<HamConfig>()
                .map_err(|e| anyhow::anyhow!("Failed to parse configuration: {e}"))?;
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
