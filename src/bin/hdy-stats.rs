use anyhow::{Context, Result};
use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::resolve_grpc_port;
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink, StatusNotify};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::{Eid, Service};
use hardy_proto::client::RemoteBpa;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::sync::OnceCell;

/// hdy-stats: Statistics utility for Hardy BPA BPv7 instances.
#[derive(Parser, Debug)]
#[command(author, version, about = "DTN statistics collector and responder for Hardy BPA", long_about = None)]
struct Args {
    /// Local gRPC port of Hardy BPA (default = 50051)
    #[arg(short, long)]
    port: Option<u16>,

    /// Use IPv6 for connecting to Hardy
    #[arg(short = '6', long)]
    ipv6: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Path to SQLite database file
    #[arg(short, long, default_value = "hdy-stats.db")]
    database: PathBuf,

    /// Retention period in days (default = 60 days, i.e. 2 months)
    #[arg(short, long, default_value_t = 60)]
    retention: u64,

    /// Log file to monitor. If not provided, journald will be used.
    #[arg(short = 'f', long)]
    log_file: Option<PathBuf>,

    /// journalctl unit name to monitor (ignored if --log-file is set)
    #[arg(short = 'u', long, default_value = "hardy-bpa")]
    journald_unit: String,

    /// Use sudo with journalctl
    #[arg(long)]
    sudo: bool,

    /// Service name to register on Hardy for stats query (default = "stats")
    #[arg(short, long, default_value = "stats")]
    service: String,

    /// Display current stats from the database and exit
    #[arg(short = 'S', long)]
    show: bool,
}

// Database helper
struct Db {
    db_path: PathBuf,
}

struct StatsReport {
    stats_24h: Vec<(String, i64)>,
    stats_week: Vec<(String, i64)>,
    stats_month: Vec<(String, i64)>,
}

impl StatsReport {
    fn to_text(&self) -> String {
        let mut out = String::new();

        out.push_str("--- 24h ---\n");
        if self.stats_24h.is_empty() {
            out.push_str("(no traffic)\n");
        } else {
            for (eid, count) in &self.stats_24h {
                out.push_str(&format!("{}: {} bundle(s)\n", eid, count));
            }
        }

        out.push_str("\n--- Week (7d) ---\n");
        if self.stats_week.is_empty() {
            out.push_str("(no traffic)\n");
        } else {
            for (eid, count) in &self.stats_week {
                out.push_str(&format!("{}: {} bundle(s)\n", eid, count));
            }
        }

        out.push_str("\n--- Month (30d) ---\n");
        if self.stats_month.is_empty() {
            out.push_str("(no traffic)\n");
        } else {
            for (eid, count) in &self.stats_month {
                out.push_str(&format!("{}: {} bundle(s)\n", eid, count));
            }
        }

        out
    }
}

fn open_connection(db_path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .context("Failed to set busy timeout on SQLite connection")?;
    Ok(conn)
}

impl Db {
    fn new(db_path: PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent().filter(|p| !p.exists()) {
            std::fs::create_dir_all(parent)
                .context("Failed to create database parent directory")?;
        }
        let conn = open_connection(&db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS bundle_traffic (
                bundle_id TEXT PRIMARY KEY,
                source_eid TEXT NOT NULL,
                received_at INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_bundle_traffic_received_at ON bundle_traffic (received_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_bundle_traffic_source_eid ON bundle_traffic (source_eid)",
            [],
        )?;
        Ok(Db { db_path })
    }

    fn record_bundle(&self, bundle_id: &str, source_eid: &str, received_at: i64) -> Result<bool> {
        let conn = open_connection(&self.db_path)?;
        let rows_affected = conn.execute(
            "INSERT OR IGNORE INTO bundle_traffic (bundle_id, source_eid, received_at) VALUES (?, ?, ?)",
            rusqlite::params![bundle_id, source_eid, received_at],
        )?;
        Ok(rows_affected > 0)
    }

    fn clean_old_records(&self, before_timestamp: i64) -> Result<usize> {
        let conn = open_connection(&self.db_path)?;
        let rows_deleted = conn.execute(
            "DELETE FROM bundle_traffic WHERE received_at < ?",
            rusqlite::params![before_timestamp],
        )?;
        Ok(rows_deleted)
    }

    fn get_stats(&self) -> Result<StatsReport> {
        let conn = open_connection(&self.db_path)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        let stats_24h = self.query_window(&conn, now - 24 * 3600)?;
        let stats_week = self.query_window(&conn, now - 7 * 24 * 3600)?;
        let stats_month = self.query_window(&conn, now - 30 * 24 * 3600)?;

        Ok(StatsReport {
            stats_24h,
            stats_week,
            stats_month,
        })
    }

    fn query_window(&self, conn: &Connection, since_timestamp: i64) -> Result<Vec<(String, i64)>> {
        let mut stmt = conn.prepare(
            "SELECT source_eid, COUNT(*) as cnt
             FROM bundle_traffic
             WHERE received_at >= ?
             GROUP BY source_eid
             ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![since_timestamp], |row| {
            let eid: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((eid, count))
        })?;

        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }
}

// Log line parsing
fn parse_bundle_log_line(line: &str) -> Option<(String, String)> {
    let is_recv = line.contains("received");
    let is_fwd = line.contains("forwarded");
    if !is_recv && !is_fwd {
        return None;
    }

    let marker = "Bundle [";
    let start_idx = line.find(marker)?;
    let id_start = start_idx + marker.len() - 1; // index of '['

    let remaining = &line[id_start..];
    let end_relative = remaining.find(']')?;
    let id_end = id_start + end_relative + 1; // index after ']'

    let bundle_id = line[id_start..id_end].to_string();

    let inner = &line[(id_start + 1)..(id_end - 1)]; // strip '[' and ']'
    let at_idx = inner.find('@')?;
    let source_eid = inner[..at_idx].trim().to_string();

    if source_eid.is_empty() {
        None
    } else {
        Some((bundle_id, source_eid))
    }
}

// Stats Service implementation
struct StatsApp {
    db: Arc<Db>,
    sink: OnceCell<Arc<Box<dyn ServiceSink>>>,
    endpoint: OnceCell<Eid>,
    verbose: bool,
}

#[async_trait]
impl BpaService for StatsApp {
    async fn on_register(&self, endpoint: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!(
                "Stats application registered successfully as Service with EID: {}",
                endpoint
            );
        }
        let _ = self.sink.set(Arc::new(sink));
        let _ = self.endpoint.set(endpoint.clone());
    }

    async fn on_unregister(&self) {
        eprintln!("Error: Stats application unregistered (connection lost). Exiting.");
        std::process::exit(1);
    }

    async fn on_receive(
        &self,
        data: Bytes,
        _expiry: time::OffsetDateTime,
    ) -> hardy_bpa::services::Result<()> {
        if self.verbose {
            eprintln!("Received stats query bundle (len: {})", data.len());
        }

        // Parse raw bundle
        let parsed =
            match hardy_bpv7::bundle::ParsedBundle::parse(&data, hardy_bpv7::bpsec::no_keys) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Failed to parse incoming bundle: {}", e);
                    return Ok(());
                }
            };
        let request_bundle = parsed.bundle;

        // Prioritize report_to, then source
        let reply_dest = if let Eid::Null = request_bundle.report_to {
            request_bundle.id.source.clone()
        } else {
            request_bundle.report_to.clone()
        };

        if let Eid::Null = reply_dest {
            eprintln!("Cannot reply: both report_to and source are Null");
            return Ok(());
        }

        if self.verbose {
            eprintln!("Generating stats report for {}", reply_dest);
        }

        // Query database for stats (run blocking queries on blocking threadpool)
        let db = self.db.clone();
        let report_res = tokio::task::spawn_blocking(move || db.get_stats()).await;

        let report = match report_res {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                eprintln!("Database error querying stats: {}", e);
                return Ok(());
            }
            Err(e) => {
                eprintln!("Task join error querying stats: {}", e);
                return Ok(());
            }
        };

        let response_text = report.to_text();
        let source_eid = self.endpoint.get().cloned().unwrap_or(Eid::Null);

        let build_res = Builder::new(source_eid, reply_dest.clone())
            .with_payload(response_text.into_bytes().into())
            .with_lifetime(std::time::Duration::from_secs(3600)) // 1 hour lifetime
            .build(CreationTimestamp::now());

        let (_resp_bundle, binbundle) = match build_res {
            Ok(val) => val,
            Err(e) => {
                eprintln!("Failed to build response bundle: {}", e);
                return Ok(());
            }
        };

        let sink = self.sink.get().cloned();
        let verbose = self.verbose;
        tokio::spawn(async move {
            if let Some(sink) = sink {
                if verbose {
                    eprintln!("Sending response bundle to {}", reply_dest);
                }
                if let Err(e) = sink.send(Bytes::from(binbundle)).await {
                    eprintln!("Failed to send stats response: {}", e);
                }
            }
        });
        Ok(())
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &hardy_bpv7::bundle::Id,
        _from: &Eid,
        _kind: StatusNotify,
        _reason: hardy_bpv7::status_report::ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
        // No-op for status notifications on sent response bundles
    }
}

async fn tail_file(path: PathBuf, tx: tokio::sync::mpsc::Sender<String>) {
    let mut offset = 0;
    let mut last_inode = 0;
    loop {
        let path_clone = path.clone();
        let current_offset = offset;
        let current_inode = last_inode;

        let result = tokio::task::spawn_blocking(move || -> Result<(Vec<String>, u64, u64), ()> {
            let metadata = std::fs::metadata(&path_clone).map_err(|_| ())?;
            let len = metadata.len();

            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;

            #[cfg(unix)]
            let inode = metadata.ino();
            #[cfg(not(unix))]
            let inode = 0;

            let mut new_offset = current_offset;
            let mut rotated = len < new_offset;
            #[cfg(unix)]
            if current_inode != 0 && inode != current_inode {
                rotated = true;
            }

            if rotated {
                new_offset = 0; // Rotated
            }
            let mut lines = Vec::new();
            let file_opt = if len > new_offset {
                std::fs::File::open(&path_clone).ok()
            } else {
                None
            };
            if let Some(mut file) = file_opt {
                use std::io::{BufRead, BufReader, Seek, SeekFrom};
                if file.seek(SeekFrom::Start(new_offset)).is_ok() {
                    let reader = BufReader::new(file);
                    for line_result in reader.lines() {
                        if let Ok(line) = line_result {
                            lines.push(line);
                        } else {
                            break;
                        }
                    }
                    new_offset = len;
                }
            }
            Ok((lines, new_offset, inode))
        })
        .await;

        match result {
            Ok(Ok((lines, new_offset, inode))) => {
                offset = new_offset;
                last_inode = inode;
                for line in lines {
                    if tx.send(line).await.is_err() {
                        return; // receiver dropped
                    }
                }
            }
            _ => {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                continue;
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }
}

async fn tail_journald(unit: String, use_sudo: bool, tx: tokio::sync::mpsc::Sender<String>) {
    loop {
        let mut cmd = if use_sudo {
            let mut c = tokio::process::Command::new("sudo");
            c.args(["journalctl", "-u", &unit, "-f", "--no-pager"]);
            c
        } else {
            let mut c = tokio::process::Command::new("journalctl");
            c.args(["-u", &unit, "-f", "--no-pager"]);
            c
        };

        cmd.kill_on_drop(true);

        match cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() {
                    let mut reader = tokio::io::BufReader::new(stdout).lines();
                    loop {
                        match reader.next_line().await {
                            Ok(Some(line)) => {
                                let _ = tx.send(line).await;
                            }
                            Ok(None) => break, // EOF, process exited
                            Err(e) => {
                                eprintln!("Error reading journalctl stdout: {}", e);
                                break;
                            }
                        }
                    }
                }
                let _ = child.kill().await; // Clean up child process
            }
            Err(e) => {
                eprintln!("Failed to spawn journalctl process: {}", e);
            }
        }

        // Wait before attempting to respawn
        eprintln!("journalctl process exited, restarting in 2 seconds...");
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
}

async fn run_log_ingestion(args: Args, db: Arc<Db>, verbose: bool) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);

    let log_file = args.log_file.clone();
    let unit = args.journald_unit.clone();
    let use_sudo = args.sudo;

    if let Some(log_file) = log_file {
        if verbose {
            eprintln!("Monitoring log file: {}", log_file.display());
        }
        tokio::spawn(async move {
            tail_file(log_file, tx).await;
        });
    } else {
        if verbose {
            eprintln!("Monitoring journald unit: {} (sudo: {})", unit, use_sudo);
        }
        tokio::spawn(async move {
            tail_journald(unit, use_sudo, tx).await;
        });
    }

    let retention_secs = args.retention * 24 * 3600;
    let mut last_cleanup = std::time::Instant::now();
    let mut last_status = std::time::Instant::now();
    let mut processed_count: u64 = 0;
    let mut matched_count: u64 = 0;

    // Initial database cleanup
    let now_ts = time::OffsetDateTime::now_utc().unix_timestamp();
    let db_clone = db.clone();
    let initial_cleanup_res = tokio::task::spawn_blocking(move || {
        db_clone.clean_old_records(now_ts - retention_secs as i64)
    })
    .await;
    match initial_cleanup_res {
        Ok(Err(e)) => eprintln!("Initial database cleanup failed: {}", e),
        Err(e) => eprintln!("Initial database cleanup task joined with error: {}", e),
        _ => {}
    }

    while let Some(line) = rx.recv().await {
        processed_count += 1;
        if let Some((bundle_id, source_eid)) = parse_bundle_log_line(&line) {
            matched_count += 1;
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let db_clone = db.clone();
            let bundle_id_clone = bundle_id.clone();
            let source_eid_clone = source_eid.clone();
            let db_res = tokio::task::spawn_blocking(move || {
                db_clone.record_bundle(&bundle_id_clone, &source_eid_clone, now)
            })
            .await;

            match db_res {
                Ok(Ok(inserted)) => {
                    if inserted && verbose {
                        eprintln!("Recorded bundle: {} from {}", bundle_id, source_eid);
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Failed to write to database: {}", e);
                }
                Err(e) => {
                    eprintln!("Task join error recording bundle: {}", e);
                }
            }
        }

        // Periodically run cleanup and log progress (every 60 seconds)
        if last_cleanup.elapsed() >= std::time::Duration::from_secs(60) {
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let db_clone = db.clone();
            let cleanup_res = tokio::task::spawn_blocking(move || {
                db_clone.clean_old_records(now - retention_secs as i64)
            })
            .await;
            match cleanup_res {
                Ok(Err(e)) => eprintln!("Periodic database cleanup failed: {}", e),
                Err(e) => eprintln!("Periodic database cleanup task joined with error: {}", e),
                _ => {}
            }
            last_cleanup = std::time::Instant::now();
        }

        if last_status.elapsed() >= std::time::Duration::from_secs(60) {
            if verbose {
                eprintln!(
                    "Log Ingestion Progress: read {} lines, matched/recorded {} bundle events",
                    processed_count, matched_count
                );
            }
            last_status = std::time::Instant::now();
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize database
    let db = Arc::new(
        Db::new(args.database.clone()).context("Failed to open/initialize SQLite database")?,
    );

    if args.show {
        let report = db
            .get_stats()
            .context("Failed to query stats from database")?;
        print!("{}", report.to_text());
        return Ok(());
    }

    // gRPC address setup
    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let port_str = resolve_grpc_port(args.port);
    let grpc_addr = format!("http://{}:{}", localhost, port_str);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let remote_bpa = RemoteBpa::new(grpc_addr);
    let app = Arc::new(StatsApp {
        db: db.clone(),
        sink: OnceCell::new(),
        endpoint: OnceCell::new(),
        verbose: args.verbose,
    });

    let service_id = if let Ok(num) = args.service.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(args.service.clone().into())
    };

    let registered_eid = remote_bpa
        .register_service(service_id, app.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Service registration failed: {e}"))?;

    eprintln!("Listening for stats requests on: {}", registered_eid);

    // Run log ingestion concurrently with the main thread
    let verbose = args.verbose;
    tokio::spawn(async move {
        if let Err(e) = run_log_ingestion(args, db, verbose).await {
            eprintln!("Log ingestion loop exited with error: {}", e);
        }
    });

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await?;
    eprintln!("\nShutting down...");

    if let Some(sink) = app.sink.get() {
        sink.unregister().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_line() {
        let line_received = "2026-07-12T08:33:49.123456Z DEBUG hardy_bpa::dispatcher::report: Bundle [ipn:1.0 @ 2026-07-12 08:33:49.123 +00:00:00 seq 1] received";
        let parsed_recv = parse_bundle_log_line(line_received).unwrap();
        assert_eq!(
            parsed_recv.0,
            "[ipn:1.0 @ 2026-07-12 08:33:49.123 +00:00:00 seq 1]"
        );
        assert_eq!(parsed_recv.1, "ipn:1.0");

        let line_forwarded = "2026-07-12T08:33:49.123456Z DEBUG hardy_bpa::dispatcher::report: Bundle [dtn://node1/service @ (No clock) seq 42 fragment 0/100] forwarded";
        let parsed_fwd = parse_bundle_log_line(line_forwarded).unwrap();
        assert_eq!(
            parsed_fwd.0,
            "[dtn://node1/service @ (No clock) seq 42 fragment 0/100]"
        );
        assert_eq!(parsed_fwd.1, "dtn://node1/service");

        let line_other = "Reaper task complete";
        assert!(parse_bundle_log_line(line_other).is_none());
    }

    #[tokio::test]
    async fn test_tail_file_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let log_file = temp.path().join("test.log");
        std::fs::write(&log_file, "line1\nline2\nlonger_line\n").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let path_clone = log_file.clone();

        let handle = tokio::spawn(async move {
            tail_file(path_clone, tx).await;
        });

        let test_fut = async {
            assert_eq!(rx.recv().await.unwrap(), "line1");
            assert_eq!(rx.recv().await.unwrap(), "line2");
            assert_eq!(rx.recv().await.unwrap(), "longer_line");

            // Simulate rotation: delete and recreate with a shorter file to force rotation detection
            // even if the OS reuses the same inode.
            std::fs::remove_file(&log_file).unwrap();
            std::fs::write(&log_file, "line3\nline4\n").unwrap();

            assert_eq!(rx.recv().await.unwrap(), "line3");
            assert_eq!(rx.recv().await.unwrap(), "line4");
        };

        tokio::time::timeout(std::time::Duration::from_secs(5), test_fut)
            .await
            .expect("test_tail_file_rotation timed out (potential deadlock)");

        handle.abort();
    }
}
