use bytes::Bytes;
use clap::Parser;
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Application, ApplicationSink, StatusNotify};
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
}

struct PrintApp {
    sink: OnceCell<Box<dyn ApplicationSink>>,
    verbose: bool,
}

#[async_trait]
impl Application for PrintApp {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ApplicationSink>) {
        if self.verbose {
            eprintln!("Application registered successfully with EID: {}", source);
        }
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        if self.verbose {
            eprintln!("Application unregistered");
        }
    }

    async fn on_receive(
        &self,
        source: Eid,
        _expiry: time::OffsetDateTime,
        _ack_requested: bool,
        payload: Bytes,
    ) {
        let text = String::from_utf8_lossy(&payload);
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "From: {}", source);
        let _ = writeln!(stdout, "{}", text);
    }

    async fn on_status_notify(
        &self,
        bundle_id: &hardy_bpv7::bundle::Id,
        from: &Eid,
        kind: StatusNotify,
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
    let grpc_addr = format!("http://{}:{}", localhost, args.port);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let remote_bpa = RemoteBpa::new(grpc_addr);
    let app = Arc::new(PrintApp {
        sink: OnceCell::new(),
        verbose: args.verbose,
    });

    let service_id = if let Ok(num) = args.service.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(args.service.clone().into())
    };

    let registered_eid = remote_bpa
        .register_application(service_id, app.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Application registration failed: {e}"))?;

    eprintln!("Listening for bundles on: {}", registered_eid);

    // Wait for Ctrl+C to exit
    tokio::signal::ctrl_c().await?;
    eprintln!("\nShutting down...");

    if let Some(sink) = app.sink.get() {
        sink.unregister().await;
    }

    Ok(())
}
