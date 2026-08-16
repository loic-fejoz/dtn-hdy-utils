use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::resolve_grpc_port;
use dtn_hdy_utils::security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink};
use hardy_bpv7::eid::{Eid, Service};
use hardy_proto::client::RemoteBpa;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::OnceCell;

/// A Bundle Protocol 7 Incoming Trigger Utility for Delay Tolerant Networking interacting with Hardy
#[derive(Parser, Debug)]
#[command(author, version, about = "Incoming trigger utility for Hardy BPA", long_about = None)]
struct Args {
    /// Local gRPC port of Hardy BPA (default = 50051)
    #[arg(short, long)]
    port: Option<u16>,

    /// Use IPv6
    #[arg(short = '6', long)]
    ipv6: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Specify local endpoint, e.g. 'incoming', or a group endpoint '7'
    #[arg(short, long)]
    endpoint: String,

    /// Just print the message
    #[arg(long)]
    print: bool,

    /// Command to execute for incoming bundles, param1 = source, param2 = payload file
    #[arg(short, long, default_value = "echo")]
    command: String,

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

struct TriggerApp {
    sink: OnceCell<Box<dyn ServiceSink>>,
    verbose: bool,
    print: bool,
    command: String,
    keystore: KeyStore,
    policy: VerifyPolicy,
    shutting_down: std::sync::atomic::AtomicBool,
}

fn write_temp_file(data: &[u8], verbose: bool) -> anyhow::Result<NamedTempFile> {
    let mut data_file = NamedTempFile::new()?;
    data_file.write_all(data)?;
    data_file.flush()?;
    if verbose {
        eprintln!("[*] data file: {}", data_file.path().display());
    }
    Ok(data_file)
}

fn execute_cmd(
    command: &str,
    data_file: NamedTempFile,
    source: &str,
    verbose: bool,
) -> anyhow::Result<()> {
    let fname_param = format!("{}", data_file.path().display());

    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c");
    let full_script = format!("{} \"$1\" \"$2\"", command);
    cmd.arg(full_script);
    cmd.arg("--"); // placeholder for $0
    cmd.arg(source);
    cmd.arg(&fname_param);

    if verbose {
        eprintln!(
            "[*] Executing: /bin/sh -c {:?} -- {} {}",
            command, source, fname_param
        );
    }

    let output = cmd.output()?;

    if !output.status.success() || verbose {
        eprintln!("[*] status: {}", output.status);
        std::io::stdout().lock().write_all(&output.stdout)?;
        std::io::stderr().lock().write_all(&output.stderr)?;
    }
    Ok(())
}

#[async_trait]
impl BpaService for TriggerApp {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!(
                "Trigger service registered successfully with EID: {}",
                source
            );
        }
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        if self.verbose {
            eprintln!("Trigger service unregistered");
        }
        if !self
            .shutting_down
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!("Error: Trigger service unregistered (connection lost). Exiting.");
            std::process::exit(1);
        }
    }

    async fn on_receive(
        &self,
        data: Bytes,
        _expiry: time::OffsetDateTime,
    ) -> hardy_bpa::services::Result<()> {
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
                        eprintln!("Bundle dropped due to strict verification policy.");
                        return Ok(());
                    }
                }
                VerifyResult::Unsigned => {
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "WARNING: Unsigned bundle received from {}. Dropped due to strict verification policy.",
                            source
                        );
                        return Ok(());
                    } else if self.verbose {
                        eprintln!("Received unsigned bundle from {}", source);
                    }
                }
            }
        }

        if self.verbose {
            eprintln!("[<] Received bundle from {}", source);
        }

        if self.print {
            let now = match time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
            {
                Ok(t) => t,
                Err(_) => "unknown_time".to_string(),
            };
            let text = String::from_utf8_lossy(&payload);
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "[{}] {} → {}", now, source, text);
        } else {
            let verbose = self.verbose;
            let command_str = self.command.clone();
            let source_str = source.to_string();

            // Write temporary file
            let data_file = match write_temp_file(&payload, verbose) {
                Ok(df) => df,
                Err(e) => {
                    eprintln!("[!] Error creating temporary file: {}", e);
                    return Ok(());
                }
            };

            // Run command in spawn_blocking to avoid blocking the async executor thread
            tokio::task::spawn_blocking(move || {
                if let Err(e) = execute_cmd(&command_str, data_file, &source_str, verbose) {
                    eprintln!("[!] Error executing command: {}", e);
                }
            });
        }
        Ok(())
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &hardy_bpv7::bundle::Id,
        _from: &Eid,
        _kind: hardy_bpa::services::StatusNotify,
        _reason: hardy_bpv7::status_report::ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
        // Ignored for triggering
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let port = resolve_grpc_port(args.port);

    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let grpc_addr = format!("http://{}:{}", localhost, port);

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
    let app = Arc::new(TriggerApp {
        sink: OnceCell::new(),
        verbose: args.verbose,
        print: args.print,
        command: args.command,
        keystore,
        policy,
        shutting_down: std::sync::atomic::AtomicBool::new(false),
    });

    let service_id = if let Ok(num) = args.endpoint.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(args.endpoint.clone().into())
    };

    let registered_eid = remote_bpa
        .register_service(service_id, app.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Service registration failed: {e}"))?;

    eprintln!("Listening for trigger events on: {}", registered_eid);

    // Wait for Ctrl+C to exit
    tokio::signal::ctrl_c().await?;
    app.shutting_down
        .store(true, std::sync::atomic::Ordering::Relaxed);
    eprintln!("\nShutting down trigger application...");

    if let Some(sink) = app.sink.get() {
        sink.unregister().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_temp_file() -> anyhow::Result<()> {
        let data = b"hello world";
        let temp = write_temp_file(data, true)?;

        let path = temp.path().to_path_buf();
        assert!(path.exists());

        let read_data = std::fs::read(&path)?;
        assert_eq!(read_data, data);

        // Dropping NamedTempFile deletes the file
        std::mem::drop(temp);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn test_execute_cmd_success() -> anyhow::Result<()> {
        let temp = write_temp_file(b"test data", false)?;
        // We will run a command like "true" which exits with 0
        let status = execute_cmd("true", temp, "dtn://source", false);
        assert!(status.is_ok());
        Ok(())
    }

    #[test]
    fn test_execute_cmd_with_quotes() -> anyhow::Result<()> {
        let temp = write_temp_file(b"test data", false)?;
        let status = execute_cmd("echo 'hello world'", temp, "dtn://source", false);
        assert!(status.is_ok());
        Ok(())
    }
}
