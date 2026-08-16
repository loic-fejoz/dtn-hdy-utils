use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::resolve_grpc_port;
use dtn_hdy_utils::security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink, StatusNotify};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::{Eid, Service};
use hardy_proto::client::RemoteBpa;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

/// A Bundle Protocol 7 Ping Utility for Delay Tolerant Networking interacting with Hardy.
/// Registers as a service on a local Hardy BPA instance via gRPC, sends ping
/// bundles, and measures round-trip time.
#[derive(Parser, Debug)]
#[command(author, version, about = "Send ping bundles and measure round-trip time", long_about = None)]
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

    /// Destination EID to ping (e.g. 'dtn://node2/echo' or 'ipn:2.7')
    destination: String,

    /// Number of pings to send
    #[arg(short, long)]
    count: Option<u32>,

    /// Interval between pings (e.g. '1s', '500ms')
    #[arg(short, long, default_value = "1s")]
    interval: String,

    /// Target bundle size in bytes (for MTU testing)
    #[arg(short, long)]
    size: Option<usize>,

    /// Total time limit for the session (e.g. '10s')
    #[arg(short = 'w', long)]
    timeout: Option<String>,

    /// Time to wait for responses after last ping (e.g. '10s')
    #[arg(short = 'W', long, default_value = "10s")]
    wait: String,

    /// Only show summary statistics
    #[arg(short, long)]
    quiet: bool,

    /// Hop limit (like IP TTL)
    #[arg(short = 't', long)]
    ttl: Option<u64>,

    /// Bundle lifetime in seconds (default: calculated based on count/interval/wait)
    #[arg(long)]
    lifetime: Option<u64>,

    /// Local endpoint/service to register (e.g. 'incoming', or '7'). If omitted, registers a dynamic endpoint.
    #[arg(short = 'S', long)]
    source: Option<String>,

    /// Path to keystore configuration file (defaults to ~/.config/dtn/keystore.toml)
    #[arg(long = "keystore")]
    keystore: Option<std::path::PathBuf>,

    /// Inline verification key material (string or hex)
    #[arg(long = "verify-key")]
    verify_key: Option<String>,

    /// Path to single verification key file
    #[arg(long = "verify-key-file")]
    verify_key_file: Option<String>,

    /// Verification policy for received bundles (strict, warn, or ignore) (default = "warn")
    #[arg(long = "verify-policy", default_value = "warn")]
    verify_policy: VerifyPolicy,

    /// Inline key material for bundle signing (string or hex)
    #[arg(long = "sign-key")]
    sign_key: Option<String>,

    /// Path to file containing key material for bundle signing
    #[arg(long = "sign-key-file")]
    sign_key_file: Option<String>,

    /// Security Source EID for BPSec BIB (defaults to bundle source EID)
    #[arg(long = "security-source")]
    security_source: Option<String>,
}

struct PathHop {
    node: Eid,
    elapsed: std::time::Duration,
    kind: StatusNotify,
}

struct PingState {
    sent: u32,
    received: u32,
    min_rtt: Option<std::time::Duration>,
    max_rtt: Option<std::time::Duration>,
    sum_rtt: std::time::Duration,
    sum_rtt_squared_us: u128,
    sent_times: HashMap<u32, std::time::Instant>,
    replied: HashSet<u32>,
    path_hops: HashMap<u32, Vec<PathHop>>,
    bundle_id_to_seqno: HashMap<hardy_bpv7::bundle::Id, u32>,
}

struct PingApp {
    sink: OnceCell<Box<dyn ServiceSink>>,
    verbose: bool,
    quiet: bool,
    destination: Eid,
    local_node_id: OnceCell<String>,
    local_eid: OnceCell<Eid>,
    state: Arc<Mutex<PingState>>,
    semaphore: Arc<tokio::sync::Semaphore>,
    keystore: KeyStore,
    policy: VerifyPolicy,
}

impl PingApp {
    fn print_path(&self, seq_no: u32) {
        let hops = {
            let mut s = self.state.lock().unwrap();
            s.path_hops.remove(&seq_no).unwrap_or_default()
        };

        if hops.is_empty() || self.quiet {
            return;
        }

        // Sort hops by elapsed time
        let mut sorted_hops = hops;
        sorted_hops.sort_by_key(|h| h.elapsed);

        // Group by node to print unique nodes in order
        let mut path_segments = Vec::new();
        for hop in sorted_hops {
            let kind_str = match hop.kind {
                StatusNotify::Received => "rcv",
                StatusNotify::Forwarded => "fwd",
                StatusNotify::Delivered => "dlv",
                StatusNotify::Deleted => "del",
            };
            path_segments.push(format!(
                "{} ({} {:.3}ms)",
                hop.node,
                kind_str,
                hop.elapsed.as_secs_f64() * 1000.0
            ));
        }

        println!("  path: {}", path_segments.join(" -> "));
    }
}

#[async_trait]
impl BpaService for PingApp {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!("Ping service registered successfully with EID: {}", source);
        }
        let _ = self.local_eid.set(source.clone());
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        if self.verbose {
            eprintln!("Ping service unregistered");
        }
    }

    async fn on_receive(
        &self,
        data: Bytes,
        _expiry: time::OffsetDateTime,
    ) -> hardy_bpa::services::Result<()> {
        let receive_time = std::time::Instant::now();

        let (source, payload) =
            match hardy_bpv7::bundle::ParsedBundle::parse(&data, hardy_bpv7::bpsec::no_keys) {
                Ok(parsed) => {
                    let payload_bytes = parsed
                        .bundle
                        .blocks
                        .get(&1)
                        .and_then(|b| data.get(b.payload_range()))
                        .map(|bytes| bytes.to_vec())
                        .unwrap_or_else(|| data.to_vec());
                    (parsed.bundle.id.source, payload_bytes)
                }
                Err(_) => (Eid::Null, data.to_vec()),
            };

        if self.policy != VerifyPolicy::Ignore {
            let res = verify_bundle(&data, &self.keystore);
            match res {
                VerifyResult::Valid => {
                    if self.verbose {
                        eprintln!("Signature verified successfully for source {}", source);
                    }
                }
                VerifyResult::Invalid(reason) => {
                    eprintln!(
                        "WARNING: Signature verification failed for {}: {}",
                        source, reason
                    );
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!("Ping response dropped due to strict verification policy.");
                        return Ok(());
                    }
                }
                VerifyResult::Unsigned => {
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "WARNING: Unsigned ping response received from {}. Dropped due to strict verification policy.",
                            source
                        );
                        return Ok(());
                    } else if self.verbose {
                        eprintln!("Received unsigned ping response from {}", source);
                    }
                }
            }
        }

        if source != self.destination {
            if self.verbose {
                eprintln!("Ignoring bundle from unexpected source EID '{}'", source);
            }
            return Ok(());
        }

        // Decode sequence number from CBOR payload
        let seq_no = match decode_ping_payload(&payload) {
            Some(seq) => seq,
            None => {
                if self.verbose {
                    eprintln!("Failed to parse sequence number from ping reply payload");
                }
                return Ok(());
            }
        };

        let sent_time = {
            let mut s = self.state.lock().unwrap();
            s.sent_times.remove(&seq_no)
        };

        let Some(sent_time) = sent_time else {
            if self.verbose {
                eprintln!(
                    "Ignoring unexpected ping response with sequence number {}",
                    seq_no
                );
            }
            return Ok(());
        };

        let rtt = receive_time.duration_since(sent_time);

        {
            let mut s = self.state.lock().unwrap();
            s.received += 1;
            s.replied.insert(seq_no);
            s.sum_rtt += rtt;
            let rtt_us = rtt.as_micros();
            s.sum_rtt_squared_us += rtt_us * rtt_us;
            s.min_rtt = Some(s.min_rtt.map_or(rtt, |min| min.min(rtt)));
            s.max_rtt = Some(s.max_rtt.map_or(rtt, |max| max.max(rtt)));
        }

        if !self.quiet {
            println!(
                "Reply from {}: seq={} rtt={:.3}ms",
                source,
                seq_no,
                rtt.as_secs_f64() * 1000.0
            );
        }

        self.print_path(seq_no);

        // Signal response received
        self.semaphore.add_permits(1);
        Ok(())
    }

    async fn on_status_notify(
        &self,
        bundle_id: &hardy_bpv7::bundle::Id,
        from: &Eid,
        kind: StatusNotify,
        reason: hardy_bpv7::status_report::ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
        let (seq_no, elapsed) = {
            let s = self.state.lock().unwrap();
            let seq = s.bundle_id_to_seqno.get(bundle_id).cloned();
            if let Some(seq_no) = seq {
                let elapsed = s.sent_times.get(&seq_no).map(|instant| instant.elapsed());
                (Some(seq_no), elapsed)
            } else {
                (None, None)
            }
        };

        if let (Some(seq_no), Some(elapsed)) = (seq_no, elapsed) {
            let direction = "Ping";
            let mut output = format!("{direction} {seq_no}");

            match kind {
                StatusNotify::Received => output.push_str(" received"),
                StatusNotify::Forwarded => output.push_str(" forwarded"),
                StatusNotify::Delivered => output.push_str(" delivered"),
                StatusNotify::Deleted => {
                    output.push_str(" deleted");
                    self.semaphore.add_permits(1);
                }
            }

            let local_id = self.local_node_id.get().map(|s| s.as_str()).unwrap_or("");
            if from.to_string() != local_id {
                output = format!("{output} by {from}");
            } else {
                output.push_str(" locally");
            }

            if !matches!(
                reason,
                hardy_bpv7::status_report::ReasonCode::NoAdditionalInformation
            ) {
                output = format!("{output}, {reason:?},");
            }

            output = format!("{output} after {:.3}ms", elapsed.as_secs_f64() * 1000.0);

            if !self.quiet {
                println!("{output}");
            }

            // Track path hops
            if from.to_string() != local_id {
                let mut s = self.state.lock().unwrap();
                s.path_hops.entry(seq_no).or_default().push(PathHop {
                    node: from.clone(),
                    elapsed,
                    kind,
                });
            }
        }
    }
}

fn parse_duration(s: &str) -> anyhow::Result<std::time::Duration> {
    let s = s.trim();
    if let Some(stripped) = s.strip_suffix("ms") {
        let val: u64 = stripped.parse()?;
        Ok(std::time::Duration::from_millis(val))
    } else if let Some(stripped) = s.strip_suffix('s') {
        let val: u64 = stripped.parse()?;
        Ok(std::time::Duration::from_secs(val))
    } else if let Some(stripped) = s.strip_suffix('m') {
        let val: u64 = stripped.parse()?;
        Ok(std::time::Duration::from_secs(val * 60))
    } else if let Some(stripped) = s.strip_suffix('h') {
        let val: u64 = stripped.parse()?;
        Ok(std::time::Duration::from_secs(val * 3600))
    } else {
        let val: u64 = s.parse()?;
        Ok(std::time::Duration::from_secs(val))
    }
}

fn encode_ping_payload(seq_no: u32, padding_len: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    // Array of 2 elements: 0x82
    buf.push(0x82);

    // Encode sequence number
    if seq_no < 24 {
        buf.push(seq_no as u8);
    } else if seq_no < 256 {
        buf.push(0x18);
        buf.push(seq_no as u8);
    } else if seq_no < 65536 {
        buf.push(0x19);
        buf.extend_from_slice(&(seq_no as u16).to_be_bytes());
    } else {
        buf.push(0x1a);
        buf.extend_from_slice(&seq_no.to_be_bytes());
    }

    // Encode options map
    if padding_len > 0 {
        // Map of 1 element: 0xa1
        buf.push(0xa1);
        // Key 0: 0x00
        buf.push(0x00);
        // Value: byte string of length padding_len
        if padding_len < 24 {
            buf.push(0x40 + padding_len as u8);
        } else if padding_len < 256 {
            buf.push(0x58);
            buf.push(padding_len as u8);
        } else if padding_len < 65536 {
            buf.push(0x59);
            buf.extend_from_slice(&(padding_len as u16).to_be_bytes());
        } else {
            buf.push(0x5a);
            buf.extend_from_slice(&(padding_len as u32).to_be_bytes());
        }
        buf.extend(std::iter::repeat_n(0u8, padding_len));
    } else {
        // Empty map: 0xa0
        buf.push(0xa0);
    }

    buf
}

fn decode_ping_payload(payload: &[u8]) -> Option<u32> {
    if payload.len() < 2 || payload[0] != 0x82 {
        return None;
    }
    let b = payload[1];
    let mut offset = 2;
    let seq_no = if b < 24 {
        b as u32
    } else if b == 0x18 {
        if payload.len() < 3 {
            return None;
        }
        let val = payload[2] as u32;
        offset = 3;
        val
    } else if b == 0x19 {
        if payload.len() < 4 {
            return None;
        }
        let val = u16::from_be_bytes([payload[2], payload[3]]) as u32;
        offset = 4;
        val
    } else if b == 0x1a {
        if payload.len() < 6 {
            return None;
        }
        let val = u32::from_be_bytes([payload[2], payload[3], payload[4], payload[5]]);
        offset = 6;
        val
    } else {
        return None;
    };

    let _ = offset; // quiet compiler
    Some(seq_no)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Exit codes matching Linux/BSD ping conventions:
    // Success = 0 (At least one response received)
    // NoResponse = 1 (No responses received)
    // Error = 2 (Other error)

    let port_str = resolve_grpc_port(args.port);

    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let grpc_addr = format!("http://{}:{}", localhost, port_str);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let remote_bpa = RemoteBpa::new(grpc_addr);

    let destination_eid = match args.destination.parse::<Eid>() {
        Ok(eid) => eid,
        Err(e) => {
            eprintln!("Invalid destination EID '{}': {}", args.destination, e);
            std::process::exit(2);
        }
    };

    let interval = match parse_duration(&args.interval) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Invalid interval '{}': {}", args.interval, e);
            std::process::exit(2);
        }
    };

    let session_timeout = match args.timeout.as_deref().map(parse_duration).transpose() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Invalid timeout '{:?}': {}", args.timeout, e);
            std::process::exit(2);
        }
    };

    let wait_time = match parse_duration(&args.wait) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Invalid wait time '{}': {}", args.wait, e);
            std::process::exit(2);
        }
    };

    let state = Arc::new(Mutex::new(PingState {
        sent: 0,
        received: 0,
        min_rtt: None,
        max_rtt: None,
        sum_rtt: std::time::Duration::ZERO,
        sum_rtt_squared_us: 0,
        sent_times: HashMap::new(),
        replied: HashSet::new(),
        path_hops: HashMap::new(),
        bundle_id_to_seqno: HashMap::new(),
    }));

    let semaphore = Arc::new(tokio::sync::Semaphore::new(0));

    let mut keystore = KeyStore::load_default_or(args.keystore.as_deref())?;

    if args.verify_key.is_some() || args.verify_key_file.is_some() {
        let key_mat = load_key(args.verify_key.as_deref(), args.verify_key_file.as_deref())?;
        keystore.add_key("*", &key_mat.raw);
    }

    let key_mat_opt = if args.sign_key.is_some() || args.sign_key_file.is_some() {
        Some(load_key(
            args.sign_key.as_deref(),
            args.sign_key_file.as_deref(),
        )?)
    } else {
        None
    };

    let policy = args.verify_policy;

    let app = Arc::new(PingApp {
        sink: OnceCell::new(),
        verbose: args.verbose,
        quiet: args.quiet,
        destination: destination_eid.clone(),
        local_node_id: OnceCell::new(),
        local_eid: OnceCell::new(),
        state: state.clone(),
        semaphore: semaphore.clone(),
        keystore,
        policy,
    });

    // Register service (Hardy gRPC server requires a service_id)
    let service_id = if let Some(ref src) = args.source {
        if let Ok(num) = src.parse::<u32>() {
            Service::Ipn(num)
        } else {
            Service::Dtn(src.clone().into())
        }
    } else {
        Service::Dtn(format!("dtnping-{}", std::process::id()).into())
    };

    let registered_eid = match remote_bpa.register_service(service_id, app.clone()).await {
        Ok(eid) => eid,
        Err(e) => {
            eprintln!("Service registration failed: {}", e);
            std::process::exit(2);
        }
    };

    let local_node_id = match registered_eid.clone().try_to_node_id() {
        Ok(node_id) => node_id.to_string(),
        Err(_) => String::new(),
    };

    let _ = app.local_node_id.set(local_node_id);

    let lifetime_secs = if let Some(lt) = args.lifetime {
        lt
    } else {
        let wait_secs = wait_time.as_secs();
        if let Some(count) = args.count {
            (interval.as_secs().saturating_mul(count as u64) + wait_secs).max(30)
        } else {
            session_timeout.map(|t| t.as_secs()).unwrap_or(300).max(30)
        }
    };
    let lifetime = std::time::Duration::from_secs(lifetime_secs);

    if !args.quiet {
        eprintln!("Pinging {} from {}", destination_eid, registered_eid);
    }

    let sink = app.sink.get().expect("Sink not set on registration");

    let count = args.count.unwrap_or(u32::MAX);
    let mut seq = 0;

    let mut timeout_future = Box::pin(async {
        if let Some(timeout) = session_timeout {
            tokio::time::sleep(timeout).await;
            true
        } else {
            std::future::pending::<bool>().await
        }
    });

    let mut ctrl_c_future = Box::pin(tokio::signal::ctrl_c());
    let mut interval_timer = tokio::time::interval(interval);

    loop {
        if seq >= count {
            break;
        }

        tokio::select! {
            _ = interval_timer.tick() => {
                // Determine padding
                let mut padding = 0;
                if let Some(target_size) = args.size {
                    let overhead = 50 + registered_eid.to_string().len() + destination_eid.to_string().len();
                    padding = target_size.saturating_sub(overhead);
                }

                let payload_bytes = encode_ping_payload(seq, padding);
                let payload_len = payload_bytes.len();

                let flags = hardy_bpv7::bundle::Flags {
                    report_status_time: true,
                    receipt_report_requested: true,
                    forward_report_requested: true,
                    delivery_report_requested: true,
                    delete_report_requested: true,
                    ..Default::default()
                };

                let source_eid = match app.local_eid.get() {
                    Some(eid) => eid.clone(),
                    None => {
                        eprintln!("Error: Local EID not set");
                        let _ = sink.unregister().await;
                        std::process::exit(2);
                    }
                };
                let (bundle, binbundle) = match Builder::new(source_eid, destination_eid.clone())
                    .with_payload(payload_bytes.into())
                    .with_lifetime(lifetime)
                    .with_flags(flags)
                    .build(CreationTimestamp::now())
                {
                    Ok(res) => res,
                    Err(e) => {
                        eprintln!("Error: Failed to build bundle: {}", e);
                        let _ = sink.unregister().await;
                        std::process::exit(2);
                    }
                };

                // Perform signing if requested
                let (_bundle, binbundle) = if let Some(ref key_mat) = key_mat_opt {
                    let sec_source = if let Some(ref sec_str) = args.security_source {
                        sec_str
                            .parse::<Eid>()
                            .map_err(|e| anyhow::anyhow!("Invalid security source EID: {e}"))?
                    } else {
                        bundle.id.source.clone()
                    };
                    if args.verbose {
                        eprintln!("Signing ping bundle with HMAC-SHA256...");
                    }
                    let (signed_bundle, signed_binbundle) =
                        dtn_hdy_utils::security::sign_bundle(&binbundle, key_mat, Some(sec_source))?;
                    (signed_bundle, signed_binbundle)
                } else {
                    (bundle, binbundle.into_vec())
                };

                if !args.quiet {
                    eprintln!("Sending ping {seq} ({} bytes payload)...", payload_len);
                }

                let send_time = std::time::Instant::now();
                match sink.send(Bytes::from(binbundle)).await {
                    Ok(bundle_id) => {
                        let mut s = state.lock().unwrap();
                        s.sent += 1;
                        s.sent_times.insert(seq, send_time);
                        s.bundle_id_to_seqno.insert(bundle_id, seq);
                    }
                    Err(e) => {
                        eprintln!("Error sending ping {seq}: {e}");
                    }
                }
                seq += 1;
            }
            _ = &mut timeout_future => {
                if !args.quiet {
                    eprintln!("Session timeout reached.");
                }
                break;
            }
            _ = &mut ctrl_c_future => {
                if !args.quiet {
                    eprintln!();
                }
                break;
            }
        }
    }

    // Wait for remaining responses
    let all_replied = {
        let s = state.lock().unwrap();
        s.received >= s.sent
    };

    if !all_replied {
        if !args.quiet {
            eprintln!(
                "Waiting up to {:.3}ms for responses...",
                wait_time.as_secs_f64() * 1000.0
            );
        }
        let wait_future = tokio::time::sleep(wait_time);
        tokio::select! {
            _ = wait_future => {}
            _ = tokio::signal::ctrl_c() => {
                if !args.quiet {
                    eprintln!();
                }
            }
            _ = async {
                loop {
                    let done = {
                        let s = state.lock().unwrap();
                        s.received >= s.sent
                    };
                    if done {
                        break;
                    }
                    let _ = semaphore.acquire().await;
                }
            } => {}
        }
    }

    // Print summary statistics
    let (sent, received, min_rtt, max_rtt, sum_rtt, sum_rtt_squared_us) = {
        let s = state.lock().unwrap();
        (
            s.sent,
            s.received,
            s.min_rtt,
            s.max_rtt,
            s.sum_rtt,
            s.sum_rtt_squared_us,
        )
    };

    println!();
    println!("--- {} ping statistics ---", destination_eid);

    let loss_pct = if sent > 0 {
        ((sent.saturating_sub(received)) as f64 / sent as f64) * 100.0
    } else {
        0.0
    };

    println!(
        "{} bundles transmitted, {} received, {:.1}% loss",
        sent, received, loss_pct
    );

    if received > 0 {
        let avg_rtt = sum_rtt.as_secs_f64() * 1000.0 / received as f64;
        let min_rtt_ms = min_rtt.unwrap_or(std::time::Duration::ZERO).as_secs_f64() * 1000.0;
        let max_rtt_ms = max_rtt.unwrap_or(std::time::Duration::ZERO).as_secs_f64() * 1000.0;

        let n = received as f64;
        let mean = sum_rtt.as_micros() as f64 / n;
        let variance = (sum_rtt_squared_us as f64 / n) - (mean * mean);
        let stddev_ms = if variance > 0.0 {
            variance.sqrt() / 1000.0
        } else {
            0.0
        };

        println!(
            "rtt min/avg/max/stddev = {:.3}ms/{:.3}ms/{:.3}ms/{:.3}ms",
            min_rtt_ms, avg_rtt, max_rtt_ms, stddev_ms
        );
    }

    // Clean up
    let exit_code = if received > 0 { 0 } else { 1 };
    sink.unregister().await;
    std::process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(
            parse_duration("500ms").unwrap(),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(
            parse_duration("2s").unwrap(),
            std::time::Duration::from_secs(2)
        );
        assert_eq!(
            parse_duration("3m").unwrap(),
            std::time::Duration::from_secs(180)
        );
    }

    #[test]
    fn test_cbor_ping_payload() {
        let encoded = encode_ping_payload(42, 0);
        let decoded = decode_ping_payload(&encoded).unwrap();
        assert_eq!(decoded, 42);

        let encoded_padded = encode_ping_payload(12345, 100);
        let decoded_padded = decode_ping_payload(&encoded_padded).unwrap();
        assert_eq!(decoded_padded, 12345);
    }
}
