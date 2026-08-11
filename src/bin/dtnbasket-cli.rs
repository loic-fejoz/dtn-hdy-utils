use anyhow::Result;
use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::basket::*;
use dtn_hdy_utils::security::{load_key, sign_bundle};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::cla::{Cla as BpaCla, ForwardBundleResult, Sink as ClaSink};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::{Eid, NodeId};
use hardy_cbor::encode::{Encoder, Tagged, ToCbor};
use hardy_proto::client::RemoteBpa;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

#[derive(Parser, Debug)]
#[command(name = "dtnbasket-cli", about = "Forge and send DTN Basket requests")]
struct Args {
    /// gRPC port of local Hardy BPA (defaults to 50051)
    #[arg(short, long)]
    port: Option<u16>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Sender endpoint / service name (e.g. "basket_client")
    #[arg(short, long, default_value = "basket_client")]
    sender: String,

    /// Receiver/Responder EID (e.g. "dtn://node2/dtnbasket")
    #[arg(short, long)]
    receiver: String,

    /// Custom request ID (defaults to randomly generated one)
    #[arg(long = "req-id")]
    req_id: Option<String>,

    /// Custom reply-to EID (defaults to sender EID)
    #[arg(long = "reply-to")]
    reply_to: Option<String>,

    /// Default lifetime for retrieved items in seconds (default = 3600)
    #[arg(long = "default-lifetime", default_value_t = 3600)]
    default_lifetime: u64,

    /// Optional experiment tag to include in the request
    #[arg(long = "experiment-tag")]
    experiment_tag: Option<u64>,

    /// Bundle lifetime in seconds (default = 3600)
    #[arg(short, long, default_value_t = 3600)]
    lifetime: u64,

    /// Request item: GET <URI>
    #[arg(long = "get", value_name = "URI")]
    gets: Vec<String>,

    /// Request item: SEARCH <URI>
    #[arg(long = "search", value_name = "URI")]
    searches: Vec<String>,

    /// Request item: LIST <URI>
    #[arg(long = "list", value_name = "URI")]
    lists: Vec<String>,

    /// Path to a TOML file containing the full list of request items
    #[arg(short = 'f', long = "request-file")]
    request_file: Option<PathBuf>,

    /// Inline key material for signing request bundle (string or hex)
    #[arg(long = "sign-key")]
    sign_key: Option<String>,

    /// Path to file containing key material for signing request bundle
    #[arg(long = "sign-key-file")]
    sign_key_file: Option<String>,

    /// Security Source EID for BPSec BIB (defaults to bundle source EID)
    #[arg(long = "security-source")]
    security_source: Option<String>,
}

#[derive(Deserialize, Debug)]
struct RequestFileItem {
    op: Option<String>,
    uri: String,
    max_size: Option<u64>,
    accepted_formats: Option<Vec<String>>,
    have_hashes: Option<Vec<String>>,
    if_modified_since: Option<u64>,
    lifetime_override: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct RequestFile {
    items: Vec<RequestFileItem>,
}

struct SenderCla {
    sink: OnceCell<Box<dyn ClaSink>>,
}

#[async_trait]
impl BpaCla for SenderCla {
    async fn on_register(&self, sink: Box<dyn ClaSink>, _node_ids: &[NodeId]) {
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {}

    async fn forward(
        &self,
        _queue: Option<u32>,
        _cla_addr: &hardy_bpa::cla::ClaAddress,
        _bundle: Bytes,
    ) -> hardy_bpa::cla::Result<ForwardBundleResult> {
        Ok(ForwardBundleResult::Sent)
    }
}

fn normalize_eid(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s
        .strip_prefix("dtn://")
        .filter(|r| !r.is_empty() && !r.contains('/'))
    {
        return format!("dtn://{}/", rest);
    }
    s.to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Gather all request items
    let mut items = Vec::new();

    // Load from request file if provided
    if let Some(ref path) = args.request_file {
        let content = std::fs::read_to_string(path)?;
        let file_data: RequestFile = toml::from_str(&content)?;
        for file_item in file_data.items {
            let op = match file_item.op.as_deref() {
                Some("GET") | Some("get") => 0,
                Some("CHECK") | Some("check") => 1,
                Some("SEARCH") | Some("search") => 2,
                Some("CANCEL") | Some("cancel") => 3,
                Some("LIST") | Some("list") => 4,
                _ => 0,
            };

            let have_hashes = if let Some(hex_hashes) = file_item.have_hashes {
                let mut decoded_hashes = Vec::new();
                for hex_str in hex_hashes {
                    decoded_hashes.push(hex::decode(hex_str.trim())?);
                }
                Some(decoded_hashes)
            } else {
                None
            };

            items.push(RequestItem {
                op,
                uri: file_item.uri,
                max_size: file_item.max_size,
                accepted_formats: file_item.accepted_formats,
                have_hashes,
                if_modified_since: file_item.if_modified_since,
                lifetime_override: file_item.lifetime_override,
            });
        }
    }

    // Add inline items
    for get_uri in &args.gets {
        items.push(RequestItem {
            op: 0,
            uri: get_uri.clone(),
            max_size: None,
            accepted_formats: None,
            have_hashes: None,
            if_modified_since: None,
            lifetime_override: None,
        });
    }

    for search_uri in &args.searches {
        items.push(RequestItem {
            op: 2,
            uri: search_uri.clone(),
            max_size: None,
            accepted_formats: None,
            have_hashes: None,
            if_modified_since: None,
            lifetime_override: None,
        });
    }

    for list_uri in &args.lists {
        items.push(RequestItem {
            op: 4,
            uri: list_uri.clone(),
            max_size: None,
            accepted_formats: None,
            have_hashes: None,
            if_modified_since: None,
            lifetime_override: None,
        });
    }

    if items.is_empty() {
        eprintln!(
            "Error: At least one request item must be specified via --get, --search, --list, or --request-file."
        );
        std::process::exit(1);
    }

    // 2. Resolve port and gRPC address
    let port_str = if let Ok(env_port) = std::env::var("HARDY_GRPC_PORT") {
        env_port
    } else if let Ok(env_port) = std::env::var("DTN_WEB_PORT") {
        env_port
    } else if let Some(cli_port) = args.port {
        cli_port.to_string()
    } else {
        "50051".to_string()
    };

    let grpc_addr = format!("http://127.0.0.1:{}", port_str);
    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let remote_bpa = RemoteBpa::new(grpc_addr);
    let sender_cla = Arc::new(SenderCla {
        sink: OnceCell::new(),
    });

    let cla_name = format!("dtnbasket-cli-{}", std::process::id());
    let node_ids = remote_bpa
        .register_cla(cla_name, sender_cla.clone(), None)
        .await
        .map_err(|e| anyhow::anyhow!("CLA registration failed: {e}"))?;

    if args.verbose {
        eprintln!("Registered CLA with BPA node IDs: {:?}", node_ids);
    }

    // 3. Determine EIDs
    let base_node = node_ids.first();
    let source_eid = match base_node {
        Some(hardy_bpv7::eid::NodeId::Dtn(node_name)) => Eid::Dtn {
            node_name: node_name.clone(),
            service_name: args.sender.trim().to_string().into_boxed_str(),
        },
        Some(hardy_bpv7::eid::NodeId::Ipn(fqnn)) => {
            let service_number = args.sender.trim().parse::<u32>().unwrap_or(0);
            Eid::Ipn {
                fqnn: fqnn.clone(),
                service_number,
            }
        }
        _ => Eid::Null,
    };

    let destination_eid = normalize_eid(&args.receiver)
        .parse::<Eid>()
        .map_err(|e| anyhow::anyhow!("Invalid receiver EID: {e}"))?;

    let reply_to_eid = if let Some(ref reply_to_str) = args.reply_to {
        if let Ok(eid) = normalize_eid(reply_to_str).parse::<Eid>() {
            Some(eid)
        } else {
            // Resolve relative to local node EID
            match base_node {
                Some(hardy_bpv7::eid::NodeId::Dtn(node_name)) => Some(Eid::Dtn {
                    node_name: node_name.clone(),
                    service_name: reply_to_str.trim().to_string().into_boxed_str(),
                }),
                Some(hardy_bpv7::eid::NodeId::Ipn(fqnn)) => {
                    let service_number = reply_to_str.trim().parse::<u32>().unwrap_or(0);
                    Some(Eid::Ipn {
                        fqnn: fqnn.clone(),
                        service_number,
                    })
                }
                _ => None,
            }
        }
    } else {
        Some(source_eid.clone())
    };

    if args.verbose {
        eprintln!("Source EID: {}", source_eid);
        eprintln!("Destination EID: {}", destination_eid);
        if let Some(ref r) = reply_to_eid {
            eprintln!("Reply-to EID: {}", r);
        }
    }

    // 4. Construct BasketRequest
    let req_id = args.req_id.clone().unwrap_or_else(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        format!("req-{}-{}", now, std::process::id())
    });

    let request = BasketRequest {
        experiment_tag: args.experiment_tag,
        version: 1,
        req_id,
        reply_to: reply_to_eid.map(|e| e.to_string()),
        default_lifetime: Some(args.default_lifetime),
        items,
    };

    // Serialize request payload to CBOR
    let mut encoder = Encoder::new();
    let tagged_req = Tagged::<44444, _>(&request);
    tagged_req.to_cbor(&mut encoder);
    let payload_bytes = encoder.build();

    // 5. Build and send bundle
    let (bundle, binbundle) = Builder::new(source_eid, destination_eid)
        .with_payload(payload_bytes.into())
        .with_lifetime(std::time::Duration::from_secs(args.lifetime))
        .build(CreationTimestamp::now())
        .map_err(|e| anyhow::anyhow!("Failed to build request bundle: {e}"))?;

    // BPSec Signing if key is provided
    let (bundle, binbundle) = maybe_sign_bundle(bundle, binbundle.into_vec(), &args)?;

    if let Some(sink) = sender_cla.sink.get() {
        sink.dispatch(Bytes::from(binbundle), None, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to dispatch request bundle: {e}"))?;

        // Print final bundle ID on stdout
        println!("Bundle-Id: {}", bundle.id.to_key());

        if args.verbose {
            eprintln!("Result: success");
        }

        // Clean up
        sink.unregister().await;
    } else {
        return Err(anyhow::anyhow!("Failed to acquire CLA sink"));
    }

    Ok(())
}

fn maybe_sign_bundle(
    bundle: hardy_bpv7::bundle::Bundle,
    binbundle: Vec<u8>,
    args: &Args,
) -> Result<(hardy_bpv7::bundle::Bundle, Vec<u8>)> {
    if args.sign_key.is_some() || args.sign_key_file.is_some() {
        let key_mat = load_key(args.sign_key.as_deref(), args.sign_key_file.as_deref())?;
        let sec_source = if let Some(ref sec_str) = args.security_source {
            Some(
                normalize_eid(sec_str)
                    .parse::<Eid>()
                    .map_err(|e| anyhow::anyhow!("Invalid security source EID: {e}"))?,
            )
        } else {
            None
        };
        if args.verbose {
            eprintln!("Signing bundle with HMAC-SHA256...");
        }
        let (signed_bundle, signed_binbundle) = sign_bundle(&binbundle, &key_mat, sec_source)?;
        Ok((signed_bundle, signed_binbundle))
    } else {
        Ok((bundle, binbundle))
    }
}
