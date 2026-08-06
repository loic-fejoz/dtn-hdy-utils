use bytes::Bytes;
use clap::Parser;
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::cla::{Cla as BpaCla, ForwardBundleResult, Sink as ClaSink};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::{DtnNodeId, Eid};
use hardy_proto::client::RemoteBpa;
use std::io::{self, Read};
use std::sync::Arc;
use tokio::sync::OnceCell;

/// A simple Bundle Protocol 7 Send Utility for Delay Tolerant Networking interacting with Hardy
#[derive(Parser, Debug)]
#[clap(version, author, long_about = None)]
struct Args {
    /// Local gRPC port (default = 50051)
    #[clap(short, long, default_value_t = 50051)]
    port: u16,

    /// Use IPv6
    #[clap(short = '6', long)]
    ipv6: bool,

    /// Verbose output
    #[clap(short, long)]
    verbose: bool,

    /// Sets sender name (e.g. 'dtn://node1' or 'incoming')
    #[clap(short, long)]
    sender: Option<String>,

    /// Receiver EID (e.g. 'dtn://node2/incoming')
    #[clap(short, long)]
    receiver: String,

    /// File to send, if omitted, data is read from stdin till EOF
    #[clap(index = 1)]
    infile: Option<String>,

    /// Don't actually send packet, just dump the encoded one.
    #[clap(short = 'D', long)]
    dryrun: bool,

    /// Bundle lifetime in seconds (default = 3600)
    #[clap(short, long, default_value_t = 3600)]
    lifetime: u64,

    /// Inline key material for bundle signing (string or hex)
    #[clap(long = "sign-key")]
    sign_key: Option<String>,

    /// Path to file containing key material for bundle signing
    #[clap(long = "sign-key-file")]
    sign_key_file: Option<String>,

    /// Security Source EID for BPSec BIB (defaults to bundle source EID)
    #[clap(long = "security-source")]
    security_source: Option<String>,
}

struct SenderCla {
    sink: OnceCell<Box<dyn ClaSink>>,
}

#[async_trait]
impl BpaCla for SenderCla {
    async fn on_register(&self, sink: Box<dyn ClaSink>, _node_ids: &[hardy_bpv7::eid::NodeId]) {
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {}

    async fn forward(
        &self,
        _queue: Option<u32>,
        _cla_addr: &hardy_bpa::cla::ClaAddress,
        _bundle: Bytes,
    ) -> hardy_bpa::cla::Result<ForwardBundleResult> {
        // We only dispatch incoming bundles, so forward is a no-op that reports success
        Ok(ForwardBundleResult::Sent)
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

    let mut buffer = Vec::new();
    if let Some(infile) = &args.infile {
        if args.verbose {
            eprintln!("Sending {}", infile);
        }
        let mut f = std::fs::File::open(infile).expect("Error accessing file.");
        f.read_to_end(&mut buffer)
            .expect("Error reading from file.");
    } else {
        io::stdin()
            .read_to_end(&mut buffer)
            .expect("Error reading from stdin.");
    }

    if args.verbose {
        eprintln!("Sending {} bytes.", buffer.len());
    }

    let destination_eid = args.receiver.parse::<Eid>().expect("invalid receiver EID");

    if args.dryrun {
        // Build the bundle locally using hardy_bpv7::builder::Builder
        // Determine the source EID for dryrun
        let source_eid = if let Some(ref sender_str) = args.sender {
            sender_str.parse::<Eid>().unwrap_or_else(|_| Eid::Dtn {
                node_name: DtnNodeId {
                    node_name: "localhost".into(),
                },
                service_name: "".into(),
            })
        } else {
            Eid::Dtn {
                node_name: DtnNodeId {
                    node_name: "localhost".into(),
                },
                service_name: "".into(),
            }
        };

        let (bundle, binbundle) = Builder::new(source_eid, destination_eid)
            .with_payload(buffer.into())
            .with_lifetime(std::time::Duration::from_secs(args.lifetime))
            .build(CreationTimestamp::now())
            .expect("failed to build bundle");

        // Perform signing if requested
        let (bundle, binbundle) = maybe_sign_bundle(bundle, binbundle.into_vec(), &args)?;

        println!("Bundle-Id: {}", bundle.id.to_key());
        let hexstr: String = binbundle.iter().map(|b| format!("{:02x}", b)).collect();
        println!("{}", hexstr);
    } else {
        let grpc_addr = format!("http://{}:{}", localhost, port_str);
        if args.verbose {
            eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
        }

        let remote_bpa = RemoteBpa::new(grpc_addr);
        let sender_cla = Arc::new(SenderCla {
            sink: OnceCell::new(),
        });

        // Register dynamically as a CLA with a unique name to avoid collisions
        let cla_name = format!("dtnsend-{}", std::process::id());
        let node_ids = remote_bpa
            .register_cla(cla_name, sender_cla.clone(), None)
            .await
            .map_err(|e| anyhow::anyhow!("CLA registration failed: {e}"))?;

        if args.verbose {
            eprintln!("Registered CLA with BPA node IDs: {:?}", node_ids);
        }

        // Determine the source EID to build the bundle
        let source_eid = if let Some(ref sender_str) = args.sender {
            if let Ok(eid) = sender_str.parse::<Eid>() {
                eid
            } else {
                // If it is just a service number or name, resolve relative to local node EID
                let base_node = node_ids.first();
                match base_node {
                    Some(hardy_bpv7::eid::NodeId::Dtn(node_name)) => Eid::Dtn {
                        node_name: node_name.clone(),
                        service_name: sender_str.clone().into_boxed_str(),
                    },
                    Some(hardy_bpv7::eid::NodeId::Ipn(fqnn)) => {
                        let service_number = sender_str.parse::<u32>().unwrap_or(0);
                        Eid::Ipn {
                            fqnn: fqnn.clone(),
                            service_number,
                        }
                    }
                    _ => Eid::Null,
                }
            }
        } else {
            match node_ids.first() {
                Some(hardy_bpv7::eid::NodeId::Dtn(node_name)) => Eid::Dtn {
                    node_name: node_name.clone(),
                    service_name: "".into(),
                },
                Some(hardy_bpv7::eid::NodeId::Ipn(fqnn)) => Eid::Ipn {
                    fqnn: fqnn.clone(),
                    service_number: 0,
                },
                _ => Eid::Null,
            }
        };

        if args.verbose {
            eprintln!("Using source EID: {}", source_eid);
        }

        // Build the bundle locally using hardy_bpv7::builder::Builder
        let (bundle, binbundle) = Builder::new(source_eid, destination_eid)
            .with_payload(buffer.into())
            .with_lifetime(std::time::Duration::from_secs(args.lifetime))
            .build(CreationTimestamp::now())
            .expect("failed to build bundle");

        // Perform signing if requested
        let (bundle, binbundle) = maybe_sign_bundle(bundle, binbundle.into_vec(), &args)?;

        // Get the sink and send the payload
        if let Some(sink) = sender_cla.sink.get() {
            sink.dispatch(Bytes::from(binbundle), None, None)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send bundle: {e}"))?;

            println!("Bundle-Id: {}", bundle.id.to_key());

            if args.verbose {
                println!("Result: success");
                let now = time::OffsetDateTime::now_utc();
                println!(
                    "Time: {}",
                    now.format(&time::format_description::well_known::Rfc3339)
                        .unwrap()
                );
            }

            // Unregister to close cleanly
            sink.unregister().await;
        } else {
            return Err(anyhow::anyhow!("Failed to acquire CLA sink"));
        }
    }

    Ok(())
}

fn maybe_sign_bundle(
    bundle: hardy_bpv7::bundle::Bundle,
    binbundle: Vec<u8>,
    args: &Args,
) -> anyhow::Result<(hardy_bpv7::bundle::Bundle, Vec<u8>)> {
    if args.sign_key.is_some() || args.sign_key_file.is_some() {
        let key_mat = dtn_hdy_utils::security::load_key(
            args.sign_key.as_deref(),
            args.sign_key_file.as_deref(),
        )?;
        let sec_source = if let Some(ref sec_str) = args.security_source {
            Some(
                sec_str
                    .parse::<Eid>()
                    .map_err(|e| anyhow::anyhow!("Invalid security source EID: {e}"))?,
            )
        } else {
            None
        };
        if args.verbose {
            eprintln!("Signing bundle with HMAC-SHA256...");
        }
        let (signed_bundle, signed_binbundle) =
            dtn_hdy_utils::security::sign_bundle(&binbundle, &key_mat, sec_source)?;
        Ok((signed_bundle, signed_binbundle))
    } else {
        Ok((bundle, binbundle))
    }
}
