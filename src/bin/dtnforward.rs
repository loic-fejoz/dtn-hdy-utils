use anyhow::Context;
use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use dtn_hdy_utils::{NoopSenderCla, normalize_eid, resolve_grpc_port};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::{Eid, Service};
use hardy_proto::client::RemoteBpa;
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Check the nesting depth of a CBOR bundle payload to prevent infinite BIBE loops.
fn get_bundle_depth(data: &[u8]) -> usize {
    let mut depth = 0;
    let mut current_bytes = data;
    while let Ok(parsed) =
        hardy_bpv7::bundle::ParsedBundle::parse(current_bytes, hardy_bpv7::bpsec::no_keys)
    {
        depth += 1;
        if let Some(payload_block) = parsed.bundle.blocks.get(&1) {
            let range = payload_block.payload_range();
            if range.end <= current_bytes.len() {
                current_bytes = &current_bytes[range.clone()];
            } else {
                break;
            }
        } else {
            break;
        }
    }
    depth
}

/// Calculate remaining lifetime of a bundle based on age block or creation timestamp.
fn get_remaining_lifetime(bundle: &hardy_bpv7::bundle::Bundle) -> Option<std::time::Duration> {
    if let Some(age) = bundle.age {
        bundle.lifetime.checked_sub(age)
    } else if let Some(created_at) = bundle.id.timestamp.as_datetime() {
        let now = time::OffsetDateTime::now_utc();
        let age_since_creation = now - created_at;
        if age_since_creation.is_negative() {
            Some(bundle.lifetime)
        } else {
            let age_duration =
                std::time::Duration::from_secs(age_since_creation.whole_seconds() as u64);
            bundle.lifetime.checked_sub(age_duration)
        }
    } else {
        None
    }
}

/// A Delay Tolerant Networking forwarder that encapsulates incoming bundles inside outer bundles (BIBE).
#[derive(Parser, Debug)]
#[command(author, version, about = "Forward received DTN bundles wrapped in an outer bundle (BIBE)", long_about = None)]
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

    /// Service number or name to listen on (default = "incoming")
    #[arg(short, long, default_value = "incoming")]
    service: String,

    /// Destination EID to forward the outer bundle to (e.g. 'dtn://f4jxq-9/bibe')
    #[arg(short, long)]
    target: String,

    /// Custom sender EID for the outer bundle (defaults to local node EID)
    #[arg(short = 'S', long)]
    sender: Option<String>,

    /// Path to keystore configuration file for verifying inner bundles (defaults to ~/.config/dtn/keystore.toml)
    #[arg(long = "keystore")]
    keystore: Option<std::path::PathBuf>,

    /// Inline verification key material (string or hex) for inner bundles
    #[arg(long = "verify-key")]
    verify_key: Option<String>,

    /// Path to single verification key file for inner bundles
    #[arg(long = "verify-key-file")]
    verify_key_file: Option<String>,

    /// Verification policy for received inner bundles (strict, warn, or ignore) (default = "warn")
    #[arg(long = "verify-policy", default_value = "warn")]
    verify_policy: VerifyPolicy,

    /// Inline key material for signing the outer bundle (string or hex)
    #[arg(long = "sign-key")]
    sign_key: Option<String>,

    /// Path to file containing key material for signing the outer bundle
    #[arg(long = "sign-key-file")]
    sign_key_file: Option<String>,

    /// Security Source EID for BPSec BIB on the outer bundle (defaults to outer bundle source EID)
    #[arg(long = "security-source")]
    security_source: Option<String>,

    /// Outer bundle lifetime in seconds. If not set, defaults to inner bundle remaining lifetime.
    #[arg(short, long)]
    lifetime: Option<u64>,

    /// Fallback lifetime for the outer bundle if inner lifetime cannot be resolved (default = 3600)
    #[arg(long = "lifetime-default", default_value_t = 3600)]
    lifetime_default: u64,

    /// Maximum encapsulation depth to prevent forwarding loops (default = 10, set to 0 to disable)
    #[arg(long = "max-depth", default_value_t = 10)]
    max_depth: u32,

    /// Wrap as raw bundle-in-bundle without standard BIBE PDU header (admin record 64443)
    #[arg(long = "raw")]
    raw: bool,
}

struct ForwardService {
    service_sink: OnceCell<Box<dyn ServiceSink>>,
    sender_cla: Arc<NoopSenderCla>,
    target_destination: Eid,
    sender_eid: Eid,
    keystore: KeyStore,
    policy: VerifyPolicy,
    verbose: bool,
    lifetime: Option<u64>,
    lifetime_default: u64,
    max_depth: u32,
    outer_sign_key: Option<String>,
    outer_sign_key_file: Option<String>,
    outer_security_source: Option<String>,
    raw: bool,
}

#[async_trait]
impl BpaService for ForwardService {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!("Service registered successfully with EID: {}", source);
        }
        let _ = self.service_sink.set(sink);
    }

    async fn on_unregister(&self) {
        eprintln!("Error: Service unregistered (connection lost). Exiting.");
        std::process::exit(1);
    }

    async fn on_receive(&self, data: Bytes, _expiry: time::OffsetDateTime) {
        let inner_parsed =
            hardy_bpv7::bundle::ParsedBundle::parse(&data, hardy_bpv7::bpsec::no_keys);

        let inner_source = match &inner_parsed {
            Ok(parsed) => parsed.bundle.id.source.clone(),
            Err(_) => Eid::Null,
        };

        if self.verbose {
            eprintln!(
                "Received bundle of size {} bytes from inner source: {}",
                data.len(),
                inner_source
            );
        }

        if self.policy != VerifyPolicy::Ignore {
            let res = verify_bundle(&data, &self.keystore);
            match res {
                VerifyResult::Valid => {
                    if self.verbose {
                        eprintln!(
                            "Signature verified successfully for inner bundle from {}",
                            inner_source
                        );
                    }
                }
                VerifyResult::Invalid(reason) => {
                    eprintln!(
                        "WARNING: Signature verification failed for inner bundle from {}: {}",
                        inner_source, reason
                    );
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!("Bundle dropped due to strict verification policy.");
                        return;
                    }
                }
                VerifyResult::Unsigned => {
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "WARNING: Unsigned inner bundle received from {}. Dropped due to strict verification policy.",
                            inner_source
                        );
                        return;
                    } else if self.verbose {
                        eprintln!("Received unsigned inner bundle from {}", inner_source);
                    }
                }
            }
        }

        // Check nesting depth
        if self.max_depth > 0 {
            let depth = get_bundle_depth(&data);
            if depth >= self.max_depth as usize {
                eprintln!(
                    "WARNING: Drop bundle from {} due to nesting depth {} exceeding max-depth {}",
                    inner_source, depth, self.max_depth
                );
                return;
            }
        }

        let outer_lifetime = if let Some(lt) = self.lifetime {
            std::time::Duration::from_secs(lt)
        } else {
            inner_parsed
                .as_ref()
                .ok()
                .and_then(|parsed| get_remaining_lifetime(&parsed.bundle))
                .filter(|&rem| rem > std::time::Duration::ZERO)
                .unwrap_or_else(|| std::time::Duration::from_secs(self.lifetime_default))
        };

        let outer_payload = if self.raw {
            std::borrow::Cow::Borrowed(&data[..])
        } else {
            // Build standard BIBE-PDU CBOR payload
            let transmission_id: u32 = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                data.hash(&mut hasher);
                (hasher.finish() & 0x7FFFFFFF) as u32
            };
            let pdu_content = (transmission_id, 0u32, hardy_cbor::encode::Bytes(&data));
            let pdu = (64443u32, pdu_content);

            let mut encoder = hardy_cbor::encode::Encoder::new();
            encoder.emit(&pdu);
            std::borrow::Cow::Owned(encoder.build())
        };

        let (outer_bundle, outer_binbundle) =
            match Builder::new(self.sender_eid.clone(), self.target_destination.clone())
                .with_payload(outer_payload)
                .with_lifetime(outer_lifetime)
                .build(CreationTimestamp::now())
            {
                Ok(res) => res,
                Err(e) => {
                    eprintln!("ERROR: Failed to build outer bundle: {}", e);
                    return;
                }
            };

        let (outer_bundle, outer_binbundle) = match dtn_hdy_utils::security::maybe_sign_bundle(
            outer_bundle,
            outer_binbundle.into_vec(),
            self.outer_sign_key.as_deref(),
            self.outer_sign_key_file.as_deref(),
            self.outer_security_source.as_deref(),
            self.verbose,
        ) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("ERROR: Failed to sign outer bundle: {}", e);
                return;
            }
        };

        if self.verbose {
            eprintln!(
                "Forwarding bundle: inner_id={} (len={}) -> outer_id={} (len={}) to {} with lifetime {}s",
                match &inner_parsed {
                    Ok(p) => p.bundle.id.to_key(),
                    Err(_) => "unknown".to_string(),
                },
                data.len(),
                outer_bundle.id.to_key(),
                outer_binbundle.len(),
                self.target_destination,
                outer_lifetime.as_secs()
            );
        }

        if let Some(sink) = self.sender_cla.sink.get() {
            match sink
                .dispatch(Bytes::from(outer_binbundle), None, None)
                .await
            {
                Ok(_) => {
                    eprintln!(
                        "Forwarded bundle: inner_id={} -> outer_id={} to {}",
                        match &inner_parsed {
                            Ok(p) => p.bundle.id.to_key(),
                            Err(_) => "unknown".to_string(),
                        },
                        outer_bundle.id.to_key(),
                        self.target_destination
                    );
                }
                Err(e) => {
                    eprintln!("ERROR: Failed to dispatch outer bundle: {}", e);
                }
            }
        } else {
            eprintln!("ERROR: CLA sink not available to dispatch outer bundle.");
        }
    }

    async fn on_status_notify(
        &self,
        _bundle_id: &hardy_bpv7::bundle::Id,
        _from: &Eid,
        _kind: hardy_bpa::services::StatusNotify,
        _reason: hardy_bpv7::status_report::ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let port_str = resolve_grpc_port(args.port);
    let grpc_addr = format!("http://{}:{}", localhost, port_str);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let remote_bpa = RemoteBpa::new(grpc_addr.clone());
    let sender_cla = Arc::new(NoopSenderCla::default());

    // Register dynamically as a CLA to get node EIDs and send outer bundles
    let cla_name = format!("dtnforward-cla-{}", std::process::id());
    let node_ids = remote_bpa
        .register_cla(cla_name, sender_cla.clone(), None)
        .await
        .map_err(|e| anyhow::anyhow!("CLA registration failed: {e}"))?;

    if args.verbose {
        eprintln!("Registered CLA with BPA node IDs: {:?}", node_ids);
    }

    // Resolve target EID
    let target_destination = normalize_eid(&args.target)
        .parse::<Eid>()
        .context("invalid target EID")?;

    // Resolve sender EID
    let sender_eid = if let Some(ref sender_str) = args.sender {
        normalize_eid(sender_str)
            .parse::<Eid>()
            .context("invalid sender EID")?
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
            _ => {
                return Err(anyhow::anyhow!(
                    "Could not determine local node EID and no custom sender EID was specified"
                ));
            }
        }
    };

    if args.verbose {
        eprintln!("Using sender EID: {}", sender_eid);
    }

    let mut keystore = KeyStore::load_default_or(args.keystore.as_deref())?;

    if args.verify_key.is_some() || args.verify_key_file.is_some() {
        let key_mat = load_key(args.verify_key.as_deref(), args.verify_key_file.as_deref())?;
        keystore.add_key("*", &key_mat.raw);
    }

    let service = Arc::new(ForwardService {
        service_sink: OnceCell::new(),
        sender_cla: sender_cla.clone(),
        target_destination: target_destination.clone(),
        sender_eid,
        keystore,
        policy: args.verify_policy,
        verbose: args.verbose,
        lifetime: args.lifetime,
        lifetime_default: args.lifetime_default,
        max_depth: args.max_depth,
        outer_sign_key: args.sign_key,
        outer_sign_key_file: args.sign_key_file,
        outer_security_source: args.security_source,
        raw: args.raw,
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
    eprintln!("Forwarding target EID: {}", target_destination);

    // Wait for Ctrl+C to exit
    tokio::signal::ctrl_c().await?;
    eprintln!("\nShutting down...");

    // Unregister service
    if let Some(sink) = service.service_sink.get() {
        sink.unregister().await;
    }
    // Unregister CLA
    if let Some(sink) = sender_cla.sink.get() {
        sink.unregister().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardy_bpv7::creation_timestamp::CreationTimestamp;
    use std::time::Duration;

    #[test]
    fn test_get_bundle_depth() {
        let src = "dtn://node1/service".parse::<Eid>().unwrap();
        let dst = "dtn://node2/service".parse::<Eid>().unwrap();

        // 1. Unsigned payload
        let payload = b"hello".to_vec();
        // Depth of non-bundle payload should be 0
        assert_eq!(get_bundle_depth(&payload), 0);

        // 2. Build one level bundle
        let (_, bundle1_bytes) = Builder::new(src.clone(), dst.clone())
            .with_payload(payload.into())
            .with_lifetime(Duration::from_secs(3600))
            .build(CreationTimestamp::now())
            .unwrap();

        // Depth of bundle1 should be 1
        assert_eq!(get_bundle_depth(&bundle1_bytes), 1);

        // 3. Nest bundle1 inside bundle2
        let (_, bundle2_bytes) = Builder::new(src.clone(), dst.clone())
            .with_payload(std::borrow::Cow::Borrowed(&bundle1_bytes))
            .with_lifetime(Duration::from_secs(3600))
            .build(CreationTimestamp::now())
            .unwrap();

        // Depth of bundle2 should be 2
        assert_eq!(get_bundle_depth(&bundle2_bytes), 2);
    }

    #[test]
    fn test_get_remaining_lifetime() {
        let src = "dtn://node1/service".parse::<Eid>().unwrap();
        let dst = "dtn://node2/service".parse::<Eid>().unwrap();

        // Create a bundle with lifetime 3600s
        let (bundle, _) = Builder::new(src, dst)
            .with_payload(b"hello".to_vec().into())
            .with_lifetime(Duration::from_secs(3600))
            .build(CreationTimestamp::now())
            .unwrap();

        // Remaining lifetime should be roughly 3600s
        let rem = get_remaining_lifetime(&bundle).unwrap();
        assert!(rem.as_secs() <= 3600);
        assert!(rem.as_secs() >= 3590); // Allow slight execution latency
    }
}
