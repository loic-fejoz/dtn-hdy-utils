use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink};
use hardy_bpv7::eid::{Eid, Service};
use hardy_proto::client::RemoteBpa;
use std::sync::Arc;
use tokio::sync::OnceCell;

/// A simple utility to receive textual BPv7 bundles from a Hardy BPA instance and print them to stdout.
#[derive(Parser, Debug)]
#[command(author, version, about = "Simple application to print received textual DTN bundles", long_about = None)]
struct Args {
    /// Local gRPC port of Hardy BPA (default = 50051)
    #[arg(short, long, default_value_t = 50051)]
    port: u16,

    /// Use IPv6 for connecting to Hardy
    #[arg(short = '6', long)]
    ipv6: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Service number or name to listen on (default = "incoming")
    #[arg(short, long, default_value = "incoming")]
    service: String,

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
}

struct PrintService {
    sink: OnceCell<Box<dyn ServiceSink>>,
    verbose: bool,
    keystore: KeyStore,
    policy: VerifyPolicy,
}

#[async_trait]
impl BpaService for PrintService {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!("Service registered successfully with EID: {}", source);
        }
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        if self.verbose {
            eprintln!("Service unregistered");
        }
    }

    async fn on_receive(&self, data: Bytes, _expiry: time::OffsetDateTime) {
        let (source, text) =
            match hardy_bpv7::bundle::ParsedBundle::parse(&data, hardy_bpv7::bpsec::no_keys) {
                Ok(parsed) => {
                    let payload_text = parsed
                        .bundle
                        .blocks
                        .get(&1)
                        .and_then(|b| data.get(b.payload_range()))
                        .map(|bytes| String::from_utf8_lossy(bytes).to_string())
                        .unwrap_or_else(|| String::from_utf8_lossy(&data).to_string());
                    (parsed.bundle.id.source, payload_text)
                }
                Err(_) => (Eid::Null, String::from_utf8_lossy(&data).to_string()),
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
                        eprintln!("Bundle dropped due to strict verification policy.");
                        return;
                    }
                }
                VerifyResult::Unsigned => {
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "WARNING: Unsigned bundle received from {}. Dropped due to strict verification policy.",
                            source
                        );
                        return;
                    } else if self.verbose {
                        eprintln!("Received unsigned bundle from {}", source);
                    }
                }
            }
        }

        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "From: {}", source);
        let _ = writeln!(stdout, "{}", text);
    }

    async fn on_status_notify(
        &self,
        bundle_id: &hardy_bpv7::bundle::Id,
        from: &Eid,
        kind: hardy_bpa::services::StatusNotify,
        reason: hardy_bpv7::status_report::ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
        if self.verbose {
            eprintln!(
                "Status notify: bundle_id={}, from={}, kind={:?}, reason={:?}",
                bundle_id.to_key(),
                from,
                kind,
                reason
            );
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let port_str = if let Ok(env_port) = std::env::var("HARDY_GRPC_PORT") {
        env_port
    } else if let Ok(env_port) = std::env::var("DTN_WEB_PORT") {
        env_port
    } else {
        args.port.to_string()
    };
    let grpc_addr = format!("http://{}:{}", localhost, port_str);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let mut keystore = KeyStore::load_default_or(args.keystore.as_deref())?;

    if args.verify_key.is_some() || args.verify_key_file.is_some() {
        let key_mat = load_key(args.verify_key.as_deref(), args.verify_key_file.as_deref())?;
        keystore.add_key("*", &key_mat.raw);
    }

    let policy = args.verify_policy;

    let remote_bpa = RemoteBpa::new(grpc_addr);
    let service = Arc::new(PrintService {
        sink: OnceCell::new(),
        verbose: args.verbose,
        keystore,
        policy,
    });

    let service_id = if let Ok(num) = args.service.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(args.service.clone().into())
    };

    let registered_eid = remote_bpa
        .register_service(service_id, service.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Service registration failed: {e}"))?;

    eprintln!("Listening for bundles on: {}", registered_eid);

    // Wait for Ctrl+C to exit
    tokio::signal::ctrl_c().await?;
    eprintln!("\nShutting down...");

    if let Some(sink) = service.sink.get() {
        sink.unregister().await;
    }

    Ok(())
}
