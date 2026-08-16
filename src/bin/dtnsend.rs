use anyhow::Context;
use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::{NoopSenderCla, normalize_eid, resolve_grpc_port};
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::{DtnNodeId, Eid};
use hardy_proto::client::RemoteBpa;
use std::io::{self, Read};
use std::sync::Arc;

/// A simple Bundle Protocol 7 Send Utility for Delay Tolerant Networking interacting with Hardy
#[derive(Parser, Debug)]
#[clap(version, author, long_about = None)]
struct Args {
    /// Local gRPC port (default = 50051)
    #[clap(short, long)]
    port: Option<u16>,

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let port_str = resolve_grpc_port(args.port);

    let mut buffer = Vec::new();
    if let Some(infile) = &args.infile {
        if args.verbose {
            eprintln!("Sending {}", infile);
        }
        let mut f = std::fs::File::open(infile).context("Error accessing file.")?;
        f.read_to_end(&mut buffer)
            .context("Error reading from file.")?;
    } else {
        io::stdin()
            .read_to_end(&mut buffer)
            .context("Error reading from stdin.")?;
    }

    if args.verbose {
        eprintln!("Sending {} bytes.", buffer.len());
    }

    let destination_eid = normalize_eid(&args.receiver)
        .parse::<Eid>()
        .context("invalid receiver EID")?;

    if args.dryrun {
        // Build the bundle locally using hardy_bpv7::builder::Builder
        // Determine the source EID for dryrun
        let source_eid = if let Some(ref sender_str) = args.sender {
            normalize_eid(sender_str)
                .parse::<Eid>()
                .unwrap_or_else(|_| Eid::Dtn {
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
            .context("failed to build bundle")?;

        // Perform signing if requested
        let (bundle, binbundle) = dtn_hdy_utils::security::maybe_sign_bundle(
            bundle,
            binbundle.into_vec(),
            args.sign_key.as_deref(),
            args.sign_key_file.as_deref(),
            args.security_source.as_deref(),
            args.verbose,
        )?;

        println!("Bundle-Id: {}", bundle.id.to_key());
        let hexstr: String = binbundle.iter().map(|b| format!("{:02x}", b)).collect();
        println!("{}", hexstr);
    } else {
        let grpc_addr = format!("http://{}:{}", localhost, port_str);
        if args.verbose {
            eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
        }

        let remote_bpa = RemoteBpa::new(grpc_addr);
        let sender_cla = Arc::new(NoopSenderCla::default());

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
            if let Ok(eid) = normalize_eid(sender_str).parse::<Eid>() {
                eid
            } else {
                // If it is just a service number or name, resolve relative to local node EID
                let base_node = node_ids.first();
                match base_node {
                    Some(hardy_bpv7::eid::NodeId::Dtn(node_name)) => Eid::Dtn {
                        node_name: node_name.clone(),
                        service_name: sender_str.trim().to_string().into_boxed_str(),
                    },
                    Some(hardy_bpv7::eid::NodeId::Ipn(fqnn)) => {
                        let service_number = sender_str.trim().parse::<u32>().unwrap_or(0);
                        Eid::Ipn {
                            fqnn: *fqnn,
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
                    fqnn: *fqnn,
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
            .context("failed to build bundle")?;

        // Perform signing if requested
        let (bundle, binbundle) = dtn_hdy_utils::security::maybe_sign_bundle(
            bundle,
            binbundle.into_vec(),
            args.sign_key.as_deref(),
            args.sign_key_file.as_deref(),
            args.security_source.as_deref(),
            args.verbose,
        )?;

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
