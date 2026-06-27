use bytes::Bytes;
use clap::Parser;
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Application, ApplicationSink, StatusNotify};
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
    #[arg(short, long, default_value_t = 50051)]
    port: u16,

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
}

struct TriggerApp {
    sink: OnceCell<Box<dyn ApplicationSink>>,
    verbose: bool,
    print: bool,
    command: String,
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
    let mut cmd_args = command.split_whitespace();
    let program = match cmd_args.next() {
        Some(p) => p,
        None => return Ok(()),
    };

    let mut cmd = std::process::Command::new(program);
    for arg in cmd_args {
        cmd.arg(arg);
    }
    cmd.arg(source);
    cmd.arg(&fname_param);

    if verbose {
        eprintln!("[*] Executing: {:?} {} {}", cmd, source, fname_param);
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
impl Application for TriggerApp {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ApplicationSink>) {
        if self.verbose {
            eprintln!(
                "Trigger application registered successfully with EID: {}",
                source
            );
        }
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        if self.verbose {
            eprintln!("Trigger application unregistered");
        }
    }

    async fn on_receive(
        &self,
        source: Eid,
        _expiry: time::OffsetDateTime,
        _ack_requested: bool,
        payload: Bytes,
    ) {
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
                    return;
                }
            };

            // Run command in spawn_blocking to avoid blocking the async executor thread
            tokio::task::spawn_blocking(move || {
                if let Err(e) = execute_cmd(&command_str, data_file, &source_str, verbose) {
                    eprintln!("[!] Error executing command: {}", e);
                }
            });
        }
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &hardy_bpv7::bundle::Id,
        _from: &Eid,
        _kind: StatusNotify,
        _reason: hardy_bpv7::status_report::ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
        // Ignored for triggering
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Port override logic using DTN_WEB_PORT, falling back to HARDY_GRPC_PORT, then argument
    let port = if let Ok(env_port) = std::env::var("DTN_WEB_PORT") {
        env_port
    } else if let Ok(env_port) = std::env::var("HARDY_GRPC_PORT") {
        env_port
    } else {
        args.port.to_string()
    };

    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let grpc_addr = format!("http://{}:{}", localhost, port);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let remote_bpa = RemoteBpa::new(grpc_addr);
    let app = Arc::new(TriggerApp {
        sink: OnceCell::new(),
        verbose: args.verbose,
        print: args.print,
        command: args.command,
    });

    let service_id = if let Ok(num) = args.endpoint.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(args.endpoint.clone().into())
    };

    let registered_eid = remote_bpa
        .register_application(service_id, app.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Application registration failed: {e}"))?;

    eprintln!("Listening for trigger events on: {}", registered_eid);

    // Wait for Ctrl+C to exit
    tokio::signal::ctrl_c().await?;
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
}
