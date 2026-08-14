use anyhow::{Context, Result};
use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use dtn_hdy_utils::{NoopSenderCla, normalize_eid, resolve_grpc_port};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink};
use hardy_bpv7::{
    builder::Builder,
    bundle::ParsedBundle,
    eid::{Eid, Service},
};
use hardy_proto::client::RemoteBpa;
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Receive and decapsulate BIBE bundles, optionally rewriting EID aliases and preserving signatures.
#[derive(Parser, Debug)]
#[command(name = "dtnbib", version, about)]
struct Args {
    /// Local service to listen on for outer BIBE bundles (default = "bibe")
    #[arg(short, long, default_value = "bibe")]
    service: String,

    /// Target EID alias mappings (e.g. "dtn://f4jxq/=dtn://f4jxq-2/" or "dtn://f4jxq/").
    /// If only the alias prefix is provided, it is automatically mapped to the local node EID.
    #[arg(short, long, value_delimiter = ',')]
    alias: Vec<String>,

    /// Disable transit decapsulation and forwarding to third-party destinations.
    #[arg(long)]
    no_transit: bool,

    /// Keystore TOML file for signature verification
    #[arg(long)]
    keystore: Option<std::path::PathBuf>,

    /// Inline verification key (string or hex)
    #[arg(long)]
    verify_key: Option<String>,

    /// Path to file containing verification key material
    #[arg(long)]
    verify_key_file: Option<String>,

    /// Verification policy for inner bundle signature (strict, warn, ignore)
    #[arg(long = "verify-policy", value_enum, default_value_t = VerifyPolicy::Warn)]
    verify_policy: VerifyPolicy,

    /// Inline key material for signing the re-injected inner bundle (string or hex)
    #[arg(long = "sign-key")]
    sign_key: Option<String>,

    /// Path to file containing key material for signing the re-injected inner bundle
    #[arg(long = "sign-key-file")]
    sign_key_file: Option<String>,

    /// Security Source EID for the new BPSec signature (defaults to local node EID)
    #[arg(long = "security-source")]
    security_source: Option<String>,

    /// Enable verbose logging to stderr
    #[arg(short, long)]
    verbose: bool,

    /// Hardy gRPC daemon port
    #[arg(short, long)]
    port: Option<u16>,

    /// Use IPv6 address for local connection
    #[arg(short = '6', long)]
    ipv6: bool,
}

struct BibService {
    service_sink: OnceCell<Box<dyn ServiceSink>>,
    sender_cla: Arc<NoopSenderCla>,
    local_node_eid: Eid,
    alias_mappings: Vec<(String, String)>,
    no_transit: bool,
    keystore: KeyStore,
    policy: VerifyPolicy,
    verbose: bool,
    sign_key: Option<String>,
    sign_key_file: Option<String>,
    security_source: Option<String>,
}

impl BibService {
    fn parse_bibe_pdu(&self, payload_data: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
        let (inner_bundle_bytes, _) =
            hardy_cbor::decode::parse_array(payload_data, |arr, _, _| {
                let record_type: u32 = arr.parse::<(u32, bool)>().map(|(v, _)| v)?;
                if record_type != 64443 {
                    return Err(hardy_cbor::decode::Error::IncorrectType(
                        "64443".to_string(),
                        record_type.to_string(),
                    ));
                }

                let arr_offset = arr.offset();
                arr.parse_array(|content_arr, _, _| {
                    let _transmission_id: u64 =
                        content_arr.parse::<(u64, bool)>().map(|(v, _)| v)?;
                    let _retransmission_time: u64 =
                        content_arr.parse::<(u64, bool)>().map(|(v, _)| v)?;

                    let content_arr_offset = content_arr.offset();
                    let res: Result<Vec<u8>, hardy_cbor::decode::Error> =
                        content_arr.parse_value(|value, _, _| match value {
                            hardy_cbor::decode::Value::Bytes(r) => {
                                let absolute_start = arr_offset + content_arr_offset + r.start;
                                let absolute_end = arr_offset + content_arr_offset + r.end;
                                Ok(payload_data[absolute_start..absolute_end].to_vec())
                            }
                            hardy_cbor::decode::Value::ByteStream(ranges) => {
                                let mut acc = Vec::new();
                                for r in ranges {
                                    let absolute_start = arr_offset + content_arr_offset + r.start;
                                    let absolute_end = arr_offset + content_arr_offset + r.end;
                                    acc.extend_from_slice(
                                        &payload_data[absolute_start..absolute_end],
                                    );
                                }
                                Ok(acc)
                            }
                            _ => Err(hardy_cbor::decode::Error::IncorrectType(
                                "Byte String".to_string(),
                                "other".to_string(),
                            )),
                        });
                    res
                })
            })?;
        Ok(inner_bundle_bytes)
    }

    fn rewrite_eid(&self, eid: &Eid) -> Result<(Eid, bool), anyhow::Error> {
        let eid_str = eid.to_string();
        for (alias, local) in &self.alias_mappings {
            if eid_str.starts_with(alias) {
                let rewritten_str = format!("{}{}", local, &eid_str[alias.len()..]);
                let rewritten_eid = rewritten_str.parse::<Eid>()?;
                return Ok((rewritten_eid, true));
            }
        }
        Ok((eid.clone(), false))
    }
}

#[async_trait]
impl BpaService for BibService {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!("Registered service EID: {}", source);
        }
        let _ = self.service_sink.set(sink);
    }

    async fn on_unregister(&self) {
        eprintln!("Error: Service unregistered (connection lost). Exiting.");
        std::process::exit(1);
    }

    async fn on_receive(&self, data: Bytes, _expiry: time::OffsetDateTime) {
        if self.verbose {
            eprintln!("Received outer bundle CBOR of size {} bytes.", data.len());
        }

        // 1. Parse the outer bundle structure
        let outer_parsed = match ParsedBundle::parse(&data, hardy_bpv7::bpsec::no_keys) {
            Ok(parsed) => parsed,
            Err(e) => {
                eprintln!("WARNING: Failed to parse outer bundle: {}", e);
                return;
            }
        };

        // 2. Extract outer bundle payload block content
        let outer_payload = match outer_parsed.bundle.blocks.get(&1) {
            Some(block) => match block.payload(&data) {
                Some(payload) => payload,
                None => {
                    eprintln!("WARNING: Failed to extract payload from outer bundle.");
                    return;
                }
            },
            None => {
                eprintln!("WARNING: Outer bundle does not have a payload block.");
                return;
            }
        };

        // 3. Decapsulate BIBE PDU (64443) or fallback to raw bundle bytes
        let inner_bundle_bytes = match self.parse_bibe_pdu(outer_payload) {
            Ok(bytes) => {
                if self.verbose {
                    eprintln!(
                        "Successfully parsed standard BIBE PDU (Administrative Record 64443)."
                    );
                }
                bytes
            }
            Err(e) => {
                if self.verbose {
                    eprintln!(
                        "Payload is not a standard BIBE PDU ({e}). Falling back to raw bundle."
                    );
                }
                outer_payload.to_vec()
            }
        };

        // 2. Parse inner bundle structure
        let inner_parsed =
            match ParsedBundle::parse(&inner_bundle_bytes, hardy_bpv7::bpsec::no_keys) {
                Ok(parsed) => parsed,
                Err(e) => {
                    eprintln!("WARNING: Failed to parse inner bundle: {}", e);
                    return;
                }
            };

        let inner_source = inner_parsed.bundle.id.source.clone();
        let inner_dest = inner_parsed.bundle.destination.clone();

        if self.verbose {
            eprintln!(
                "Decapsulated inner bundle: id={} source={} destination={}",
                inner_parsed.bundle.id.to_key(),
                inner_source,
                inner_dest
            );
        }

        // 3. Verify original signature of the inner bundle
        let mut is_signature_valid = false;
        if self.policy != VerifyPolicy::Ignore {
            let res = verify_bundle(&inner_bundle_bytes, &self.keystore);
            match res {
                VerifyResult::Valid => {
                    is_signature_valid = true;
                    if self.verbose {
                        eprintln!(
                            "Signature verified successfully for inner bundle from {}.",
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
                        eprintln!("Inner bundle dropped due to strict verification policy.");
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
                        eprintln!("Inner bundle from {} is unsigned.", inner_source);
                    }
                }
            }
        }

        // 4. Resolve EID Aliases / Rewriting
        let (rewritten_dest, eid_was_rewritten) = match self.rewrite_eid(&inner_dest) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("ERROR: Failed to rewrite EID: {}", e);
                return;
            }
        };

        if eid_was_rewritten && self.verbose {
            eprintln!("EID rewritten: {} -> {}", inner_dest, rewritten_dest);
        }

        // 5. Transit check
        let is_local = rewritten_dest
            .to_string()
            .starts_with(&self.local_node_eid.to_string());

        if !is_local && self.no_transit {
            if self.verbose {
                eprintln!(
                    "Dropped transit inner bundle destined for {} due to --no-transit.",
                    rewritten_dest
                );
            }
            return;
        }

        // 6. Build and re-sign if necessary
        let (final_bundle, final_binbundle) = if eid_was_rewritten {
            // Extract original payload bytes from inner bundle
            let inner_payload = match inner_parsed.bundle.blocks.get(&1) {
                Some(block) => match block.payload(&inner_bundle_bytes) {
                    Some(payload) => payload,
                    None => {
                        eprintln!("WARNING: Failed to extract payload from inner bundle.");
                        return;
                    }
                },
                None => {
                    eprintln!("WARNING: Inner bundle does not have a payload block.");
                    return;
                }
            };

            // Rebuild inner bundle structure with rewritten destination EID
            let (rebuilt_bundle, rebuilt_binbundle) = match Builder::new(
                inner_parsed.bundle.id.source.clone(),
                rewritten_dest.clone(),
            )
            .with_lifetime(inner_parsed.bundle.lifetime)
            .with_flags(inner_parsed.bundle.flags)
            .with_payload(std::borrow::Cow::Borrowed(inner_payload))
            .build(inner_parsed.bundle.id.timestamp.clone())
            {
                Ok(res) => res,
                Err(e) => {
                    eprintln!("ERROR: Failed to rebuild inner bundle: {}", e);
                    return;
                }
            };

            // Determine key to use for re-signing
            let mut key_to_use = None;

            if is_signature_valid {
                if self.sign_key.is_some() || self.sign_key_file.is_some() {
                    // Specific key provided via CLI
                    match load_key(self.sign_key.as_deref(), self.sign_key_file.as_deref()) {
                        Ok(k) => key_to_use = Some(k),
                        Err(e) => {
                            eprintln!("ERROR: Failed to load sign-key: {}", e);
                            return;
                        }
                    }
                } else {
                    // Find key matching original sender in keystore
                    let matching_keys = self.keystore.find_keys(&inner_parsed.bundle.id.source);
                    if let Some(key) = matching_keys.first() {
                        if let hardy_bpv7::bpsec::key::Type::OctetSequence { key } = &key.key_type {
                            key_to_use =
                                Some(dtn_hdy_utils::security::KeyMaterial { raw: key.to_vec() });
                            if self.verbose {
                                eprintln!(
                                    "Found matching key for original sender {} in keystore. Using it to re-sign.",
                                    inner_parsed.bundle.id.source
                                );
                            }
                        }
                    } else if self.verbose {
                        eprintln!(
                            "WARNING: No matching key for original sender {} found in keystore to re-sign inner bundle.",
                            inner_parsed.bundle.id.source
                        );
                    }
                }
            }

            if let Some(key_mat) = key_to_use {
                let sec_source = self
                    .security_source
                    .as_ref()
                    .and_then(|s| s.parse::<Eid>().ok())
                    .or_else(|| Some(inner_parsed.bundle.id.source.clone()));

                if self.verbose {
                    eprintln!("Re-signing rewritten inner bundle...");
                }
                match dtn_hdy_utils::security::sign_bundle(
                    &rebuilt_binbundle.into_vec(),
                    &key_mat,
                    sec_source,
                ) {
                    Ok(res) => res,
                    Err(e) => {
                        eprintln!("ERROR: Failed to sign rebuilt inner bundle: {}", e);
                        return;
                    }
                }
            } else {
                (rebuilt_bundle, rebuilt_binbundle.into_vec())
            }
        } else {
            // No EID rewrite, keep original signed bytes
            (inner_parsed.bundle, inner_bundle_bytes)
        };

        // 7. Re-inject into local BPA
        if let Some(sink) = self.sender_cla.sink.get() {
            match sink
                .dispatch(Bytes::from(final_binbundle), None, None)
                .await
            {
                Ok(_) => {
                    eprintln!(
                        "Re-injected inner bundle: {} -> {}",
                        final_bundle.id.to_key(),
                        rewritten_dest
                    );
                }
                Err(e) => {
                    eprintln!("ERROR: Failed to re-inject bundle: {}", e);
                }
            }
        } else {
            eprintln!("ERROR: CLA sink not available to re-inject bundle.");
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

    // Validate custom security source EID at startup if provided
    if let Some(ref sec_source_str) = args.security_source {
        normalize_eid(sec_source_str)
            .parse::<Eid>()
            .context("Invalid --security-source EID format")?;
    }

    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let port_str = resolve_grpc_port(args.port);
    let grpc_addr = format!("http://{}:{}", localhost, port_str);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let remote_bpa = RemoteBpa::new(grpc_addr.clone());
    let sender_cla = Arc::new(NoopSenderCla::default());

    // Register dynamically as a CLA with a retry loop to withstand transient BPA restarts
    let cla_name = format!("dtnbib-cla-{}", std::process::id());
    let mut node_ids = None;
    for i in 0..5 {
        match remote_bpa
            .register_cla(cla_name.clone(), sender_cla.clone(), None)
            .await
        {
            Ok(ids) => {
                node_ids = Some(ids);
                break;
            }
            Err(e) => {
                if i == 4 {
                    return Err(anyhow::anyhow!(
                        "CLA registration failed after 5 attempts: {e}"
                    ));
                }
                if args.verbose {
                    eprintln!(
                        "WARNING: CLA registration failed (attempt {}): {}. Retrying in 1s...",
                        i + 1,
                        e
                    );
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
    let node_ids = node_ids.unwrap();

    if args.verbose {
        eprintln!("Registered CLA with BPA node IDs: {:?}", node_ids);
    }

    // Resolve local node EID
    let local_node_eid = match node_ids.first() {
        Some(hardy_bpv7::eid::NodeId::Dtn(node_name)) => Eid::Dtn {
            node_name: node_name.clone(),
            service_name: "".into(),
        },
        Some(hardy_bpv7::eid::NodeId::Ipn(fqnn)) => Eid::Ipn {
            fqnn: fqnn.clone(),
            service_number: 0,
        },
        _ => {
            return Err(anyhow::anyhow!("Could not determine local node EID"));
        }
    };

    if args.verbose {
        eprintln!("Local node EID: {}", local_node_eid);
    }

    // Parse alias mappings
    let mut alias_mappings = Vec::new();
    for alias_arg in &args.alias {
        if let Some(pos) = alias_arg.find('=') {
            let (alias, local) = alias_arg.split_at(pos);
            let local = &local[1..]; // skip '='
            let normalized_alias = normalize_eid(alias);
            let normalized_local = normalize_eid(local);
            alias_mappings.push((normalized_alias, normalized_local));
        } else {
            // Auto-resolve local node EID base
            let normalized_alias = normalize_eid(alias_arg);
            let local_str = local_node_eid.to_string();
            alias_mappings.push((normalized_alias, local_str));
        }
    }

    if args.verbose && !alias_mappings.is_empty() {
        eprintln!("Alias mappings configured:");
        for (alias, local) in &alias_mappings {
            eprintln!("  {} => {}", alias, local);
        }
    }

    let mut keystore = KeyStore::load_default_or(args.keystore.as_deref())?;

    if args.verify_key.is_some() || args.verify_key_file.is_some() {
        let key_mat = load_key(args.verify_key.as_deref(), args.verify_key_file.as_deref())?;
        keystore.add_key("*", &key_mat.raw);
    }

    let service = Arc::new(BibService {
        service_sink: OnceCell::new(),
        sender_cla: sender_cla.clone(),
        local_node_eid,
        alias_mappings,
        no_transit: args.no_transit,
        keystore,
        policy: args.verify_policy,
        verbose: args.verbose,
        sign_key: args.sign_key,
        sign_key_file: args.sign_key_file,
        security_source: args.security_source,
    });

    let service_id = if let Ok(num) = args.service.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(args.service.clone().into())
    };

    let mut registered_eid = None;
    for i in 0..5 {
        match remote_bpa
            .register_service(service_id.clone(), service.clone())
            .await
        {
            Ok(eid) => {
                registered_eid = Some(eid);
                break;
            }
            Err(e) => {
                if i == 4 {
                    return Err(anyhow::anyhow!(
                        "Service registration failed after 5 attempts: {e}"
                    ));
                }
                if args.verbose {
                    eprintln!(
                        "WARNING: Service registration failed (attempt {}): {}. Retrying in 1s...",
                        i + 1,
                        e
                    );
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
    let registered_eid = registered_eid.unwrap();

    eprintln!("Listening for BIBE bundles on: {}", registered_eid);

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

    #[test]
    fn test_cbor_bibe_pdu_roundtrip() {
        let inner_bundle_data = b"hello nested bundle".to_vec();

        // Build standard BIBE-PDU
        let transmission_id: u32 = 42;
        let pdu_content = (
            transmission_id,
            0u32,
            hardy_cbor::encode::Bytes(&inner_bundle_data),
        );
        let pdu = (64443u32, pdu_content);

        let mut encoder = hardy_cbor::encode::Encoder::new();
        encoder.emit(&pdu);
        let bibe_pdu_bytes = encoder.build();

        // Parse standard BIBE-PDU using our helper
        let service = BibService {
            service_sink: OnceCell::new(),
            sender_cla: Arc::new(NoopSenderCla::default()),
            local_node_eid: "dtn://node2/".parse().unwrap(),
            alias_mappings: vec![],
            no_transit: false,
            keystore: KeyStore::empty(),
            policy: VerifyPolicy::Ignore,
            verbose: false,
            sign_key: None,
            sign_key_file: None,
            security_source: None,
        };

        let parsed_bytes = service.parse_bibe_pdu(&bibe_pdu_bytes).unwrap();
        assert_eq!(parsed_bytes, inner_bundle_data);
    }
}
