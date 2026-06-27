use clap::{Parser, Subcommand};
use hardy_bpv7::bundle::Id as Bpv7Id;
use rusqlite::Connection;
use sha1::{Digest, Sha1};
use sqlx::PgPool;
use std::path::PathBuf;

/// A simple Bundle Protocol 7 Query Utility for Delay Tolerant Networking interacting with Hardy
#[derive(Parser, Debug)]
#[clap(version, author, long_about = None)]
struct Args {
    /// Local gRPC/web port (unused, kept for dtn7 CLI compatibility)
    #[clap(short, long, default_value_t = 3000)]
    port: u16,

    /// Use IPv6 (unused, kept for dtn7 CLI compatibility)
    #[clap(short = '6', long)]
    ipv6: bool,

    /// Path to the Hardy config file, overriding default location
    #[clap(short, long)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List registered endpoint IDs
    Eids,
    /// List known peers
    Peers,
    /// List bundles on node
    Bundles {
        /// Verbose output includes bundle destination
        #[clap(short, long)]
        verbose: bool,
        /// Just print hash digest of bundles known
        #[clap(short, long)]
        digest: bool,
        /// Filter for bundles with source or destination address
        #[clap(short, long)]
        addr: Option<String>,
    },
    /// List bundles status in store
    Store,
    /// General dtnd info
    Info,
    /// Local node id
    Nodeid,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct HardyConfig {
    #[serde(default)]
    node_ids: Vec<String>,
    #[serde(default)]
    storage: StorageConfig,
}

impl Default for HardyConfig {
    fn default() -> Self {
        Self {
            node_ids: vec!["ipn:1.0".to_string(), "dtn://localhost/".to_string()],
            storage: StorageConfig::default(),
        }
    }
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
struct StorageConfig {
    #[serde(default)]
    metadata: MetadataConfig,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum MetadataConfig {
    #[serde(rename = "memory")]
    Memory,
    #[serde(rename = "sqlite")]
    Sqlite {
        #[serde(default = "default_db_dir", rename = "db-dir")]
        db_dir: PathBuf,
        #[serde(default = "default_db_name", rename = "db-name")]
        db_name: String,
    },
    #[serde(rename = "postgres")]
    Postgres {
        #[serde(rename = "database-url")]
        database_url: String,
    },
}

impl Default for MetadataConfig {
    fn default() -> Self {
        MetadataConfig::Sqlite {
            db_dir: default_db_dir(),
            db_name: default_db_name(),
        }
    }
}

fn default_db_name() -> String {
    "metadata.db".to_string()
}

fn default_db_dir() -> PathBuf {
    directories::ProjectDirs::from("dtn", "Hardy", "hardy-sqlite-storage").map_or_else(
        || {
            #[cfg(unix)]
            return PathBuf::from("/var/spool/hardy-sqlite-storage");
            #[cfg(not(unix))]
            return PathBuf::from(".");
        },
        |dirs| dirs.cache_dir().to_path_buf(),
    )
}

fn resolve_config(explicit_path: Option<PathBuf>) -> HardyConfig {
    let (config_file, is_required) = if let Some(path) = explicit_path {
        (path, true)
    } else if let Ok(env_val) = std::env::var("HARDY_BPA_SERVER_CONFIG_FILE") {
        (PathBuf::from(env_val), true)
    } else {
        #[cfg(unix)]
        let path = PathBuf::from("/etc/hardy/bpa");
        #[cfg(not(unix))]
        let path = PathBuf::from("bpa");
        (path, false)
    };

    let builder = ::config::Config::builder();
    let builder = if config_file.extension().is_some() {
        builder.add_source(::config::File::from(config_file).required(is_required))
    } else {
        builder.add_source(
            ::config::File::with_name(&config_file.to_string_lossy()).required(is_required),
        )
    };

    let builder = builder.add_source(
        ::config::Environment::with_prefix("HARDY_BPA_SERVER")
            .prefix_separator("_")
            .separator("__")
            .convert_case(::config::Case::Kebab)
            .try_parsing(true),
    );

    match builder.build() {
        Ok(config_val) => match config_val.try_deserialize::<HardyConfig>() {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Warning: Failed to deserialize config: {e}. Using defaults.");
                HardyConfig::default()
            }
        },
        Err(e) => {
            if is_required {
                eprintln!("Error: Failed to load required config file: {e}");
                std::process::exit(1);
            } else {
                HardyConfig::default()
            }
        }
    }
}

#[derive(Debug, Clone)]
struct BundleInfo {
    id: String,
    source: String,
    destination: String,
    creation_time: u64,
    size: usize,
    status: &'static str,
}

fn query_sqlite(db_path: &std::path::Path) -> anyhow::Result<Vec<BundleInfo>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn
        .prepare("SELECT bundle_id, bundle, status_code FROM bundles WHERE bundle IS NOT NULL")?;
    let rows = stmt.query_map([], |row| {
        let id_bytes: Vec<u8> = row.get(0)?;
        let bundle_bytes: Vec<u8> = row.get(1)?;
        let status_code: i64 = row.get(2)?;
        Ok((id_bytes, bundle_bytes, status_code))
    })?;

    let mut list = Vec::new();
    for r in rows {
        let (id_bytes, bundle_bytes, status_code) = r?;
        let bpv7_id: Bpv7Id = match serde_json::from_slice(&id_bytes) {
            Ok(parsed_id) => parsed_id,
            Err(_) => continue,
        };
        let bpa_bundle: hardy_bpa::bundle::Bundle = match serde_json::from_slice(&bundle_bytes) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let dest_eid = bpa_bundle.bundle.destination.to_string();
        let size = bpa_bundle
            .bundle
            .blocks
            .values()
            .map(|b| b.extent.end)
            .max()
            .unwrap_or(0);

        let creation_time = bpv7_id
            .timestamp
            .creation_time()
            .map_or(0, |t| t.millisecs());
        let status = match status_code {
            0 => "New",
            1 => "Waiting",
            2 => "ForwardPending",
            3 => "AduFragment",
            4 => "Dispatching",
            5 => "WaitingForService",
            _ => "Unknown",
        };

        list.push(BundleInfo {
            id: bpv7_id.to_key(),
            source: bpv7_id.source.to_string(),
            destination: dest_eid,
            creation_time,
            size,
            status,
        });
    }

    Ok(list)
}

async fn query_postgres(database_url: &str) -> anyhow::Result<Vec<BundleInfo>> {
    let pool = PgPool::connect(database_url).await?;
    let rows = sqlx::query_as::<_, (String, String, Vec<u8>)>(
        "SELECT bundles.bundle_id, metadata.status::text, metadata.bundle \
         FROM bundles \
         JOIN metadata ON bundles.id = metadata.id",
    )
    .fetch_all(&pool)
    .await?;

    let mut list = Vec::new();
    for (id_str, status_str, json_bytes) in rows {
        let bpv7_id = match Bpv7Id::from_key(&id_str) {
            Ok(parsed_id) => parsed_id,
            Err(_) => continue,
        };

        let bpa_bundle: hardy_bpa::bundle::Bundle = match serde_json::from_slice(&json_bytes) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let dest_eid = bpa_bundle.bundle.destination.to_string();
        let size = bpa_bundle
            .bundle
            .blocks
            .values()
            .map(|b| b.extent.end)
            .max()
            .unwrap_or(0);

        let creation_time = bpv7_id
            .timestamp
            .creation_time()
            .map_or(0, |t| t.millisecs());
        let status = match status_str.as_str() {
            "new" => "New",
            "waiting" => "Waiting",
            "forward_pending" => "ForwardPending",
            "adu_fragment" => "AduFragment",
            "dispatching" => "Dispatching",
            "waiting_for_service" => "WaitingForService",
            _ => "Unknown",
        };

        list.push(BundleInfo {
            id: id_str,
            source: bpv7_id.source.to_string(),
            destination: dest_eid,
            creation_time,
            size,
            status,
        });
    }

    Ok(list)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = resolve_config(args.config);

    // Fetch bundle states offline via DB access
    let bundles_info = match &config.storage.metadata {
        MetadataConfig::Memory => {
            eprintln!("BPA is configured with in-memory storage. No offline database to query.");
            Vec::new()
        }
        MetadataConfig::Sqlite { db_dir, db_name } => {
            let db_path = db_dir.join(db_name);
            if !db_path.exists() {
                eprintln!(
                    "Warning: SQLite database file not found at {}. Returning empty.",
                    db_path.display()
                );
                Vec::new()
            } else {
                match query_sqlite(&db_path) {
                    Ok(list) => list,
                    Err(e) => {
                        eprintln!("Error querying SQLite database: {e}");
                        Vec::new()
                    }
                }
            }
        }
        MetadataConfig::Postgres { database_url } => match query_postgres(database_url).await {
            Ok(list) => list,
            Err(e) => {
                eprintln!("Error querying PostgreSQL database: {e}");
                Vec::new()
            }
        },
    };

    match &args.cmd {
        Commands::Nodeid => {
            println!("Local node ID:");
            if let Some(first_id) = config.node_ids.first() {
                println!("{}", first_id);
            } else {
                println!("dtn:none");
            }
        }
        Commands::Eids => {
            println!("Listing registered endpoint IDs:");
            println!(
                "Information not available: dynamic endpoint registrations are kept in memory by the Hardy BPA server."
            );
            println!("Please refer to TICKETS-FOR-HARDY.md for details.");
        }
        Commands::Peers => {
            println!("Listing of known peers:");
            println!(
                "Information not available: active peer discovery tables are kept in memory by the Hardy BPA server."
            );
            println!("Please refer to TICKETS-FOR-HARDY.md for details.");
        }
        Commands::Store => {
            println!("Listing of bundles status in store:");
            let status_list: Vec<String> = bundles_info
                .iter()
                .map(|b| format!("{} {{{}}}", b.id, b.status))
                .collect();
            println!("{}", serde_json::to_string_pretty(&status_list)?);
        }
        Commands::Info => {
            println!("Daemon info:");
            let mut incoming = 0;
            let mut outgoing = 0;
            for b in &bundles_info {
                match b.status {
                    "New" | "Waiting" | "WaitingForService" => incoming += 1,
                    "ForwardPending" | "Dispatching" => outgoing += 1,
                    _ => {}
                }
            }
            let stats = serde_json::json!({
                "incoming": incoming,
                "dups": 0,
                "outgoing": outgoing,
                "delivered": 0,
                "broken": 0
            });
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Commands::Bundles {
            verbose,
            digest,
            addr,
        } => {
            println!("Listing of bundles in store:");
            let filtered: Vec<&BundleInfo> = bundles_info
                .iter()
                .filter(|b| {
                    if let Some(filter_addr) = addr {
                        b.source.contains(filter_addr) || b.destination.contains(filter_addr)
                    } else {
                        true
                    }
                })
                .collect();

            if *digest {
                let mut ids: Vec<String> = filtered.iter().map(|b| b.id.clone()).collect();
                ids.sort();
                let mut hasher = Sha1::new();
                for id in ids {
                    hasher.update(id.as_bytes());
                }
                let hash_str = format!("{:x}", hasher.finalize());
                println!("{}", hash_str);
            } else if *verbose {
                let meta_list: Vec<String> = filtered
                    .iter()
                    .map(|b| {
                        format!(
                            "{} {} {} {}",
                            b.source, b.destination, b.creation_time, b.size
                        )
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&meta_list)?);
            } else {
                let id_list: Vec<String> = filtered.iter().map(|b| b.id.clone()).collect();
                println!("{}", serde_json::to_string_pretty(&id_list)?);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_load_config(content: &str, name: &str) -> HardyConfig {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, content).unwrap();
        let config = resolve_config(Some(path.clone()));
        let _ = std::fs::remove_file(path);
        config
    }

    #[test]
    fn test_sqlite_config_deserialization() {
        let yaml = r#"
storage:
  metadata:
    type: sqlite
    db-dir: /tmp/my-db-dir
    db-name: test.db
"#;
        let config = test_load_config(yaml, "test_sqlite_config.yaml");
        match config.storage.metadata {
            MetadataConfig::Sqlite { db_dir, db_name } => {
                assert_eq!(db_dir, std::path::PathBuf::from("/tmp/my-db-dir"));
                assert_eq!(db_name, "test.db");
            }
            _ => panic!("Expected Sqlite metadata config"),
        }
    }

    #[test]
    fn test_postgres_config_deserialization() {
        let yaml = r#"
storage:
  metadata:
    type: postgres
    database-url: "postgres://localhost/db"
"#;
        let config = test_load_config(yaml, "test_postgres_config.yaml");
        match config.storage.metadata {
            MetadataConfig::Postgres { database_url } => {
                assert_eq!(database_url, "postgres://localhost/db");
            }
            _ => panic!("Expected Postgres metadata config"),
        }
    }

    #[test]
    fn test_memory_config_deserialization() {
        let yaml = r#"
storage:
  metadata:
    type: memory
"#;
        let config = test_load_config(yaml, "test_memory_config.yaml");
        match config.storage.metadata {
            MetadataConfig::Memory => {}
            _ => panic!("Expected Memory metadata config"),
        }
    }

    #[test]
    fn test_query_sqlite_success() -> anyhow::Result<()> {
        let db_path = std::env::temp_dir().join("test_dtnquery_query.db");
        if db_path.exists() {
            let _ = std::fs::remove_file(&db_path);
        }

        let conn = Connection::open(&db_path)?;
        conn.execute(
            "CREATE TABLE bundles (
                id INTEGER PRIMARY KEY,
                bundle_id BLOB NOT NULL UNIQUE,
                expiry TEXT NOT NULL,
                received_at TEXT NOT NULL,
                status_code INTEGER,
                status_param1 INTEGER,
                status_param2 INTEGER,
                status_param3 TEXT,
                bundle BLOB
            ) STRICT;",
            [],
        )?;

        // Construct mock bundle IDs
        let bpv7_id = Bpv7Id {
            source: "dtn://my-source/".parse().unwrap(),
            timestamp: hardy_bpv7::creation_timestamp::CreationTimestamp::now(),
            fragment_info: None,
        };
        let id_bytes = serde_json::to_vec(&bpv7_id)?;

        // Construct mock bundle
        let mut bpa_bundle = hardy_bpa::bundle::Bundle {
            bundle: hardy_bpv7::bundle::Bundle {
                id: bpv7_id.clone(),
                flags: Default::default(),
                crc_type: hardy_bpv7::crc::CrcType::None,
                destination: "dtn://my-destination/".parse().unwrap(),
                report_to: "dtn://my-report/".parse().unwrap(),
                lifetime: core::time::Duration::from_secs(3600),
                previous_node: None,
                age: None,
                hop_count: None,
                blocks: std::collections::HashMap::new(),
            },
            metadata: Default::default(),
        };

        // Add a dummy block to blocks map to verify size calculation
        let dummy_block = hardy_bpv7::block::Block {
            extent: 0..128,
            ..Default::default()
        };
        bpa_bundle.bundle.blocks.insert(1, dummy_block);

        let bundle_bytes = serde_json::to_vec(&bpa_bundle)?;

        // Insert into database
        conn.execute(
            "INSERT INTO bundles (bundle_id, expiry, received_at, status_code, bundle) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id_bytes, "2026-06-18T12:00:00Z", "2026-06-18T11:00:00Z", 1, bundle_bytes],
        )?;

        // Run query_sqlite
        let results = query_sqlite(&db_path)?;
        assert_eq!(results.len(), 1);
        let info = &results[0];
        assert_eq!(info.id, bpv7_id.to_key());
        assert_eq!(info.source, "dtn://my-source/");
        assert_eq!(info.destination, "dtn://my-destination/");
        assert_eq!(info.size, 128);
        assert_eq!(info.status, "Waiting");

        // Clean up
        let _ = std::fs::remove_file(&db_path);
        Ok(())
    }
}
