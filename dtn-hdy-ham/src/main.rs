//! CLI binary entry point for `dtn-hdy-ham`.

use clap::{Parser, Subcommand};
use dtn_hdy_ham::{HamConfig, HamServer};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "dtn-hdy-ham",
    author,
    version,
    about = "Delay Tolerant Networking BPv7 bundle processing agent dedicated to amateur radio operators"
)]
struct Cli {
    /// Path to YAML/TOML configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Override local node EID (e.g. 'dtn://n0call.ampr.org/' or 'ipn:1.0')
    #[arg(short, long)]
    node_id: Option<String>,

    /// Run with in-memory volatile storage (overrides config)
    #[arg(long)]
    in_memory: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print an annotated example configuration file
    GenerateConfig,
    /// Run the interactive configuration wizard
    Init {
        /// Optional path where to save the configuration file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

const EXAMPLE_CONFIG: &str = r#"# dtn-hdy-ham: Hamradio DTN BPv7 Node Configuration
# EID format can be DTN scheme (e.g. dtn://CALLSIGN.ampr.org/) or IPN scheme (e.g. ipn:NODE.0)
node-id: "dtn://n0call.ampr.org/"

# Storage settings (bundle storage directory & embedded SQLite database)
storage:
  data-dir: "~/.local/share/dtn-hdy-ham"
  in-memory: false

# Local gRPC endpoint for CLI tools (dtnsend, dtnprint, dtnping, etc.)
grpc:
  enabled: true
  address: "[::]:50051"

# Convergence Layer Adapters
clas:
  # 1. TCPCLv4 over Hamnet / 44Net / Internet (RFC 9174)
  - name: tcpcl-44net-gateway
    type: tcpclv4
    address: "[::]:4556" # Local listening port
    require-tls: false   # Disabled for amateur radio compliance (no encryption)
    peers:
      - "44.27.131.233:4556"

  # 2. HQFBP over KISS (TNC via TCP or Serial)
  - name: kiss-tnc-vhf
    type: hqfbp
    callsign: "N0CALL-1"
    transport:
      type: tcp # For Direwolf / soundmodem KISS server
      address: "127.0.0.1:8001"
      port: 0

  # 3. Licklider Transmission Protocol (LTP RFC 5326 over UDP or KISS)
  - name: ltp-udp-cislunar
    type: ltp
    engine-id: 1001
    transport:
      type: udp
      bind-addr: "0.0.0.0:1113"

# Static routing rules
routes:
  - pattern: "dtn://*/*"
    cla: "tcpcl-44net-gateway"
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if let Some(Commands::GenerateConfig) = cli.command {
        print!("{EXAMPLE_CONFIG}");
        return Ok(());
    }

    if let Some(Commands::Init { output }) = cli.command {
        dtn_hdy_ham::run_interactive_wizard(output.or(cli.config))?;
        return Ok(());
    }

    let mut config = if let Some(config_file) = HamConfig::find_config(cli.config.as_deref()) {
        HamConfig::load(Some(&config_file))?
    } else {
        dtn_hdy_ham::run_interactive_wizard(cli.config.clone())?
    };

    if let Some(nid) = cli.node_id {
        config.node_id = nid;
    }
    if cli.in_memory {
        config.storage.in_memory = true;
    }

    info!("Starting Hardy Ham BPA with node-id: {}", config.node_id);
    let server = HamServer::new(config).await?;
    server.run().await?;

    Ok(())
}
