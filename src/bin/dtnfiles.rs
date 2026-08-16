use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::resolve_grpc_port;
use dtn_hdy_utils::security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink};
use hardy_bpv7::eid::{Eid, Service};
use hardy_proto::client::RemoteBpa;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

/// A utility to receive BPv7 bundles from a Hardy BPA instance and save them as files in a directory.
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Saves incoming BPv7 bundles to a directory with auto-detected file extensions",
    long_about = None
)]
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

    /// Service number or name to listen on (default = "files")
    #[arg(short, long, default_value = "files")]
    service: String,

    /// Target directory to save incoming bundles
    #[arg(index = 1, required = true)]
    dir: PathBuf,

    /// Path to keystore configuration file (defaults to ~/.config/dtn/keystore.toml)
    #[arg(long = "keystore")]
    keystore: Option<PathBuf>,

    /// Inline verification key material (string or hex)
    #[arg(long = "verify-key")]
    verify_key: Option<String>,

    /// Path to single verification key file
    #[arg(long = "verify-key-file")]
    verify_key_file: Option<String>,

    /// Verification policy for received bundles (strict, warn, or ignore) (default = "warn")
    #[arg(long = "verify-policy", default_value = "warn")]
    verify_policy: VerifyPolicy,

    /// Save bundle metadata as JSON alongside the payload (filename: <bundle-id>-metadata.json)
    #[arg(short = 'm', long = "metadata")]
    metadata: bool,
}

struct FilesService {
    sink: OnceCell<Box<dyn ServiceSink>>,
    verbose: bool,
    dir: PathBuf,
    keystore: KeyStore,
    policy: VerifyPolicy,
    metadata: bool,
}

fn detect_extension(data: &[u8]) -> &'static str {
    // Check DTN Basket Response
    if hardy_cbor::decode::parse::<dtn_hdy_utils::basket::BasketResponse>(data).is_ok() {
        return "basket.json";
    }

    let len = data.len();
    if len >= 4 {
        // Check PNG
        if data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
            return "png";
        }
        // Check JPEG
        if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
            return "jpg";
        }
        // Check GIF
        if data[0] == 0x47 && data[1] == 0x49 && data[2] == 0x46 {
            return "gif";
        }
        // Check ZIP (PK\x03\x04)
        if data[0] == 0x50 && data[1] == 0x4B && data[2] == 0x03 && data[3] == 0x04 {
            return "zip";
        }
        // Check WEBP (RIFFxxxxWEBP)
        if len >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            return "webp";
        }
        // Check BMP
        if data[0] == 0x42 && data[1] == 0x4D {
            return "bmp";
        }
        // Check OGG
        if data[0] == 0x4F && data[1] == 0x67 && data[2] == 0x67 && data[3] == 0x53 {
            return "ogg";
        }
        // Check MP3
        if (&data[0..3] == b"ID3") || (data[0] == 0xFF && (data[1] & 0xE0) == 0xE0) {
            return "mp3";
        }
        // Check Gzip/tar.gz (\x1f\x8b)
        if data[0] == 0x1F && data[1] == 0x8B {
            return "tar.gz";
        }
        // Check M4A/MP4 (ftyp at offset 4)
        if len >= 8 && &data[4..8] == b"ftyp" {
            return "m4a";
        }
    } else if len >= 3 {
        // Check JPEG
        if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
            return "jpg";
        }
        // Check GIF
        if data[0] == 0x47 && data[1] == 0x49 && data[2] == 0x46 {
            return "gif";
        }
        // Check MP3 ID3
        if &data[0..3] == b"ID3" {
            return "mp3";
        }
        // Check BMP
        if data[0] == 0x42 && data[1] == 0x4D {
            return "bmp";
        }
        // Check Gzip/tar.gz (\x1f\x8b)
        if data[0] == 0x1F && data[1] == 0x8B {
            return "tar.gz";
        }
        // Check MP3 frame sync
        if data[0] == 0xFF && (data[1] & 0xE0) == 0xE0 {
            return "mp3";
        }
    } else if len >= 2 {
        // Check BMP
        if data[0] == 0x42 && data[1] == 0x4D {
            return "bmp";
        }
        // Check Gzip/tar.gz (\x1f\x8b)
        if data[0] == 0x1F && data[1] == 0x8B {
            return "tar.gz";
        }
        // Check MP3 frame sync
        if data[0] == 0xFF && (data[1] & 0xE0) == 0xE0 {
            return "mp3";
        }
    }

    // Check if valid text / Markdown
    if let Ok(text) = std::str::from_utf8(data) {
        if text.starts_with('#') || text.contains("**") || text.contains("* ") {
            return "md";
        }
        return "txt";
    }

    "bin"
}

#[async_trait]
impl BpaService for FilesService {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!("Service registered successfully with EID: {}", source);
        }
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        eprintln!("Error: Service unregistered (connection lost). Exiting.");
        std::process::exit(1);
    }

    async fn on_receive(
        &self,
        data: Bytes,
        _expiry: time::OffsetDateTime,
    ) -> hardy_bpa::services::Result<()> {
        let (source, bundle_id_str, payload, bundle_meta) =
            match hardy_bpv7::bundle::ParsedBundle::parse(&data, hardy_bpv7::bpsec::no_keys) {
                Ok(parsed) => {
                    let payload_bytes = parsed
                        .bundle
                        .blocks
                        .get(&1)
                        .and_then(|b| data.get(b.payload_range()))
                        .map(|bytes| bytes.to_vec())
                        .unwrap_or_else(|| data.to_vec());
                    let bundle_id_str = parsed.bundle.id.to_key();
                    let source = parsed.bundle.id.source.clone();

                    let creation_time = parsed.bundle.id.timestamp.as_datetime().and_then(|dt| {
                        dt.format(&time::format_description::well_known::Rfc3339)
                            .ok()
                    });

                    let meta_val = serde_json::json!({
                        "bundle_id": bundle_id_str,
                        "source": parsed.bundle.id.source.to_string(),
                        "destination": parsed.bundle.destination.to_string(),
                        "report_to": parsed.bundle.report_to.to_string(),
                        "creation_time": creation_time,
                        "sequence_number": parsed.bundle.id.timestamp.sequence_number(),
                        "lifetime_seconds": parsed.bundle.lifetime.as_secs(),
                        "is_fragment": parsed.bundle.id.fragment_info.is_some(),
                        "fragment_offset": parsed.bundle.id.fragment_info.as_ref().map(|f| f.offset),
                        "total_adu_length": parsed.bundle.id.fragment_info.as_ref().map(|f| f.total_adu_length),
                    });

                    (source, bundle_id_str, payload_bytes, Some(meta_val))
                }
                Err(_) => {
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&data);
                    let hash = hex::encode(hasher.finalize());
                    (Eid::Null, hash, data.to_vec(), None)
                }
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

        let ext = detect_extension(&payload);
        let safe_bundle_id = bundle_id_str.replace(['/', '\\'], "_");
        let filename = format!("{}.{}", safe_bundle_id, ext);
        let dest_path = self.dir.join(filename);

        let to_write = if ext == "basket.json" {
            hardy_cbor::decode::parse::<dtn_hdy_utils::basket::BasketResponse>(&payload)
                .map_err(|e| e.to_string())
                .and_then(|resp| serde_json::to_string_pretty(&resp).map_err(|e| e.to_string()))
                .map(|json_str| json_str.into_bytes())
                .unwrap_or_else(|_| payload.clone())
        } else {
            payload.clone()
        };

        if self.verbose {
            eprintln!("Saving payload from {} to {}", source, dest_path.display());
        }

        if let Err(e) = std::fs::write(&dest_path, &to_write) {
            eprintln!(
                "ERROR: Failed to write bundle payload to {}: {}",
                dest_path.display(),
                e
            );
        } else {
            eprintln!("Saved bundle payload to {}", dest_path.display());

            if let (true, Some(meta_val)) = (self.metadata, bundle_meta) {
                let meta_filename = format!("{}-metadata.json", safe_bundle_id);
                let meta_path = self.dir.join(meta_filename);
                if let Ok(meta_str) = serde_json::to_string_pretty(&meta_val) {
                    if let Err(e) = std::fs::write(&meta_path, meta_str) {
                        eprintln!(
                            "ERROR: Failed to write bundle metadata to {}: {}",
                            meta_path.display(),
                            e
                        );
                    } else if self.verbose {
                        eprintln!("Saved bundle metadata to {}", meta_path.display());
                    }
                }
            }
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
        // Ignored
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

    let mut keystore = KeyStore::load_default_or(args.keystore.as_deref())?;

    if args.verify_key.is_some() || args.verify_key_file.is_some() {
        let key_mat = load_key(args.verify_key.as_deref(), args.verify_key_file.as_deref())?;
        keystore.add_key("*", &key_mat.raw);
    }

    let policy = args.verify_policy;

    // Create the target directory if it does not exist
    if !args.dir.exists() {
        if args.verbose {
            eprintln!("Creating target directory: {}", args.dir.display());
        }
        std::fs::create_dir_all(&args.dir)?;
    }

    let remote_bpa = RemoteBpa::new(grpc_addr);
    let service = Arc::new(FilesService {
        sink: OnceCell::new(),
        verbose: args.verbose,
        dir: args.dir.clone(),
        keystore,
        policy,
        metadata: args.metadata,
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

    eprintln!("Listening for files bundles on: {}", registered_eid);

    // Wait for Ctrl+C to exit
    tokio::signal::ctrl_c().await?;
    eprintln!("\nShutting down...");

    if let Some(sink) = service.sink.get() {
        sink.unregister().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_extension() {
        assert_eq!(detect_extension(b"\x89PNG"), "png");
        assert_eq!(detect_extension(b"\xff\xd8\xff"), "jpg");
        assert_eq!(detect_extension(b"GIF89a"), "gif");
        assert_eq!(detect_extension(b"RIFF\0\0\0\0WEBP"), "webp");
        assert_eq!(detect_extension(b"BMxxxx"), "bmp");
        assert_eq!(detect_extension(b"OggS"), "ogg");
        assert_eq!(detect_extension(b"ID3xxx"), "mp3");
        assert_eq!(detect_extension(b"\xff\xe0\0"), "mp3");
        assert_eq!(detect_extension(b"\0\0\0\0ftypm4a"), "m4a");
        assert_eq!(detect_extension(b"PK\x03\x04xxxx"), "zip");
        assert_eq!(detect_extension(b"\x1f\x8bxxxx"), "tar.gz");

        // Text / Markdown detection
        assert_eq!(detect_extension(b"# Title\nHello"), "md");
        assert_eq!(detect_extension(b"Some **bold** text"), "md");
        assert_eq!(detect_extension(b"List:\n* item 1"), "md");
        assert_eq!(detect_extension(b"Just plain text."), "txt");

        // Fallback
        assert_eq!(detect_extension(b"\xff\x00\xff\x00"), "bin");
    }

    #[test]
    fn test_detect_basket_response() {
        use dtn_hdy_utils::basket::{BasketResponse, ItemMetadata, ItemResponse};
        use hardy_cbor::encode::{self, ToCbor};

        let response = BasketResponse {
            experiment_tag: None,
            version: 1,
            req_id: "test_request".to_string(),
            items: vec![ItemResponse {
                item_idx: 0,
                coap_status: 69, // 2.05 Content
                metadata: Some(ItemMetadata {
                    hash: vec![1, 2, 3, 4],
                    size: Some(100),
                    mime_type: Some("image/png".to_string()),
                    uri: Some("dtn://node/file.png".to_string()),
                    last_modified: Some(12345678),
                }),
                diagnostic: None,
            }],
        };

        let mut encoder = encode::Encoder::new();
        response.to_cbor(&mut encoder);
        let cbor_bytes = encoder.build();

        assert_eq!(detect_extension(&cbor_bytes), "basket.json");

        // Verify JSON conversion logic
        let parsed: BasketResponse = hardy_cbor::decode::parse(&cbor_bytes).unwrap();
        let json_str = serde_json::to_string_pretty(&parsed).unwrap();
        assert!(json_str.contains("test_request"));
        assert!(json_str.contains("image/png"));
        assert!(json_str.contains("01020304")); // serialized hash should be hex
    }

    #[test]
    fn test_filename_sanitization() {
        let bundle_id_str = "goIBeBsvL2Y0anhxLTIvYmFza2V0X3Rlc3RfcmVwbHmCGwAAAMOF8fPOGgAEkcc";
        let safe_bundle_id = bundle_id_str.replace(['/', '\\'], "_");
        assert!(!safe_bundle_id.contains('/'));
        assert!(!safe_bundle_id.contains('\\'));
        assert_eq!(
            safe_bundle_id,
            "goIBeBsvL2Y0anhxLTIvYmFza2V0X3Rlc3RfcmVwbHmCGwAAAMOF8fPOGgAEkcc".replace('/', "_")
        );

        let traversal_bundle_id = "../../../../etc/passwd";
        let safe_traversal = traversal_bundle_id.replace(['/', '\\'], "_");
        assert_eq!(safe_traversal, ".._.._.._.._etc_passwd");
    }
}
