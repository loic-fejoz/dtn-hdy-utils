use anyhow::Result;
use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::security::{KeyStore, VerifyPolicy, VerifyResult, load_key, verify_bundle};
use hardy_bpa::async_trait;
use hardy_bpa::bpa::BpaRegistration;
use hardy_bpa::services::{Service as BpaService, ServiceSink};
use hardy_bpv7::builder::Builder;
use hardy_bpv7::creation_timestamp::CreationTimestamp;
use hardy_bpv7::eid::{Eid, Service};
use hardy_cbor::decode;
use hardy_cbor::encode::{self, Tagged};
use hardy_proto::client::RemoteBpa;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

// Custom self-signed certificate verifier for insecure TLS mode
#[derive(Debug)]
struct SelfSignedVerifier;

impl ServerCertVerifier for SelfSignedVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

/// A Bundle Protocol 7 Basket Protocol Service for Delay Tolerant Networking interacting with Hardy.
/// Implements RFC draft-f4jxq-dtn-basket-00.
#[derive(Parser, Debug)]
#[command(author, version, about = "DTN Basket Protocol Responder Service", long_about = None)]
struct Args {
    /// Path to TOML configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Local gRPC port of Hardy BPA (default = 50051)
    #[arg(short, long)]
    port: Option<u16>,

    /// Use IPv6 for connecting to Hardy
    #[arg(short = '6', long)]
    ipv6: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Service endpoint to register (default = "dtnbasket")
    #[arg(short, long)]
    service: Option<String>,

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

    /// Inline key material for response bundle signing (string or hex)
    #[arg(long = "sign-key")]
    sign_key: Option<String>,

    /// Path to file containing key material for response bundle signing
    #[arg(long = "sign-key-file")]
    sign_key_file: Option<String>,

    /// Security Source EID for BPSec BIB (defaults to bundle source EID)
    #[arg(long = "security-source")]
    security_source: Option<String>,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct Config {
    #[serde(default)]
    insecure_tls: bool,
    #[serde(default = "default_grpc_port")]
    bpa_grpc_port: u16,
    #[serde(default = "default_service_name")]
    service_name: String,
    #[serde(default)]
    allowed_dirs: Vec<PathBuf>,
    #[serde(default)]
    mappings: HashMap<String, PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            insecure_tls: false,
            bpa_grpc_port: 50051,
            service_name: "dtnbasket".to_string(),
            allowed_dirs: Vec::new(),
            mappings: HashMap::new(),
        }
    }
}

fn default_grpc_port() -> u16 {
    50051
}

fn default_service_name() -> String {
    "dtnbasket".to_string()
}

use dtn_hdy_utils::basket::*;

// Client fetching logic and security checks

fn build_tls_connector(insecure_tls: bool) -> Result<tokio_rustls::TlsConnector> {
    let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let client_config_builder = ClientConfig::builder();

    let client_config = if insecure_tls {
        let mut config = client_config_builder
            .with_root_certificates(root_store)
            .with_no_client_auth();
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(SelfSignedVerifier));
        config
    } else {
        client_config_builder
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };

    Ok(tokio_rustls::TlsConnector::from(Arc::new(client_config)))
}

fn parse_gemini_url(url: &str) -> Result<(String, u16, String)> {
    let url = url.trim();
    if !url.starts_with("gemini://") {
        return Err(anyhow::anyhow!("URL does not start with gemini://"));
    }
    let rest = &url["gemini://".len()..];
    let (authority, _path_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.find(':') {
        Some(idx) => {
            let host = &authority[..idx];
            let port = authority[idx + 1..].parse::<u16>()?;
            (host.to_string(), port)
        }
        None => (authority.to_string(), 1965),
    };
    Ok((host, port, url.to_string()))
}

async fn fetch_gemini(
    url_str: &str,
    connector: &tokio_rustls::TlsConnector,
    max_size: u64,
) -> Result<(Vec<u8>, String)> {
    let (host, port, full_url) = parse_gemini_url(url_str)?;

    let tcp_stream = tokio::net::TcpStream::connect((host.as_str(), port)).await?;

    let server_name = rustls::pki_types::ServerName::try_from(host.clone())
        .map_err(|_| anyhow::anyhow!("Invalid server name: {}", host))?
        .to_owned();

    let mut tls_stream = connector.connect(server_name, tcp_stream).await?;

    // Send request
    let request = format!("{}\r\n", full_url);
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tls_stream.write_all(request.as_bytes()).await?;
    tls_stream.flush().await?;

    // Read response header up to \r\n
    let mut header_bytes = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        let n = tls_stream.read(&mut buf).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("Connection closed before header received"));
        }
        header_bytes.push(buf[0]);
        if header_bytes.ends_with(b"\r\n") {
            break;
        }
        if header_bytes.len() > 1026 {
            return Err(anyhow::anyhow!("Response header too long"));
        }
    }

    let header_str = String::from_utf8_lossy(&header_bytes[..header_bytes.len() - 2]);
    let parts: Vec<&str> = header_str.splitn(2, ' ').collect();
    if parts.is_empty() {
        return Err(anyhow::anyhow!("Invalid Gemini response header"));
    }
    let status_str = parts[0];
    let meta = if parts.len() > 1 { parts[1] } else { "" };

    if status_str.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid Gemini status code: {}",
            status_str
        ));
    }
    let status_char = status_str
        .chars()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Empty status string"))?;

    match status_char {
        '2' => {
            // Success
            let mime = if meta.is_empty() {
                "text/gemini; charset=utf-8".to_string()
            } else {
                meta.to_string()
            };
            // Read body up to max_size
            let mut body = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = tls_stream.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
                if body.len() as u64 > max_size {
                    return Err(anyhow::anyhow!("Payload too large"));
                }
            }
            Ok((body, mime))
        }
        '3' => {
            // Redirect
            Err(anyhow::anyhow!("REDIRECT: {}", meta))
        }
        _ => Err(anyhow::anyhow!(
            "Gemini error status {}: {}",
            status_str,
            meta
        )),
    }
}

fn check_file_path(path: &Path, allowed_dirs: &[PathBuf]) -> bool {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    for dir in allowed_dirs {
        if dir
            .canonicalize()
            .map(|dir_c| path.starts_with(dir_c))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn guess_mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()) {
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("md") | Some("markdown") => "text/markdown; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("cbor") => "application/cbor",
        Some("xml") => "application/xml",
        _ => "application/octet-stream",
    }
}

fn is_safe_host(host: &str) -> bool {
    let host = host.to_lowercase();
    if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "0.0.0.0" {
        return false;
    }
    if host.starts_with("10.") || host.starts_with("192.168.") {
        return false;
    }
    if host.starts_with("172.") {
        let is_private_172 = host
            .split('.')
            .nth(1)
            .and_then(|s| s.parse::<u8>().ok())
            .map(|second_octet| (16..=31).contains(&second_octet))
            .unwrap_or(false);
        if is_private_172 {
            return false;
        }
    }
    true
}

async fn fetch_resource(
    uri: &str,
    client: &reqwest::Client,
    connector: &tokio_rustls::TlsConnector,
    config: &Config,
    max_size: u64,
) -> Result<(Vec<u8>, String)> {
    let mut current_uri = uri.to_string();
    let mut redirect_count = 0;

    loop {
        if current_uri.starts_with("http://") || current_uri.starts_with("https://") {
            let url = reqwest::Url::parse(&current_uri)?;
            if let Some(host) = url.host_str().filter(|h| !is_safe_host(h)) {
                return Err(anyhow::anyhow!(
                    "Forbidden host (SSRF protection): {}",
                    host
                ));
            }

            let res = client.get(&current_uri).send().await?;
            let status = res.status();
            if status.is_success() {
                let mime = res
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();

                if res
                    .content_length()
                    .map(|len| len > max_size)
                    .unwrap_or(false)
                {
                    return Err(anyhow::anyhow!("Payload too large"));
                }

                let bytes = res.bytes().await?;
                if bytes.len() as u64 > max_size {
                    return Err(anyhow::anyhow!("Payload too large"));
                }
                return Ok((bytes.to_vec(), mime));
            } else if status.is_redirection() {
                if redirect_count >= 5 {
                    return Err(anyhow::anyhow!("Too many redirects"));
                }
                redirect_count += 1;
                let location = res
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .ok_or_else(|| anyhow::anyhow!("Redirect without Location header"))?;

                let base = reqwest::Url::parse(&current_uri)?;
                let resolved = base.join(location)?;

                let scheme = resolved.scheme();
                if scheme != "http" && scheme != "https" && scheme != "gemini" {
                    return Err(anyhow::anyhow!("Forbidden redirect scheme: {}", scheme));
                }

                current_uri = resolved.to_string();
                continue;
            } else {
                return Err(anyhow::anyhow!("HTTP error: {}", status));
            }
        } else if current_uri.starts_with("gemini://") {
            let (host, _, _) = parse_gemini_url(&current_uri)?;
            if !is_safe_host(&host) {
                return Err(anyhow::anyhow!(
                    "Forbidden host (SSRF protection): {}",
                    host
                ));
            }

            match fetch_gemini(&current_uri, connector, max_size).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    let err_msg = e.to_string();
                    if let Some(redirect_meta) = err_msg.strip_prefix("REDIRECT: ") {
                        if redirect_count >= 5 {
                            return Err(anyhow::anyhow!("Too many redirects"));
                        }
                        redirect_count += 1;
                        let base = reqwest::Url::parse(&current_uri)?;
                        let resolved = base.join(redirect_meta.trim())?;

                        let scheme = resolved.scheme();
                        if scheme != "http" && scheme != "https" && scheme != "gemini" {
                            return Err(anyhow::anyhow!("Forbidden redirect scheme: {}", scheme));
                        }

                        current_uri = resolved.to_string();
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        } else if let Some(file_path_str) = current_uri.strip_prefix("file://") {
            let path = PathBuf::from(file_path_str);
            if !check_file_path(&path, &config.allowed_dirs) {
                return Err(anyhow::anyhow!(
                    "Access denied or file not allowed: {}",
                    file_path_str
                ));
            }
            let bytes = std::fs::read(&path)?;
            if bytes.len() as u64 > max_size {
                return Err(anyhow::anyhow!("Payload too large"));
            }
            let mime = guess_mime_type(&path).to_string();
            return Ok((bytes, mime));
        } else if let Some(mapped_path) = config.mappings.get(&current_uri) {
            let canonical = mapped_path.canonicalize()?;
            let bytes = std::fs::read(&canonical)?;
            if bytes.len() as u64 > max_size {
                return Err(anyhow::anyhow!("Payload too large"));
            }
            let mime = guess_mime_type(&canonical).to_string();
            return Ok((bytes, mime));
        } else {
            return Err(anyhow::anyhow!(
                "Unsupported URI scheme or mapping: {}",
                current_uri
            ));
        }
    }
}

// Regex search and List compilation

fn perform_search(query: &str, config: &Config) -> Vec<(String, PathBuf)> {
    let mut results = Vec::new();
    let re = match regex::RegexBuilder::new(query)
        .case_insensitive(true)
        .build()
    {
        Ok(r) => r,
        Err(_) => return results,
    };

    // Search mappings
    for (uri, path) in &config.mappings {
        let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if re.is_match(uri) || re.is_match(filename) {
            results.push((uri.clone(), path.clone()));
        }
    }

    // Search allowed_dirs
    for dir in &config.allowed_dirs {
        let mut dirs_to_visit = vec![(dir.clone(), 0)];
        while let Some((current_dir, depth)) = dirs_to_visit.pop() {
            if depth > 3 {
                continue;
            }
            let read_dir = match std::fs::read_dir(&current_dir) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for entry in read_dir {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = entry.path();
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if metadata.is_dir() {
                    let dir_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                    if dir_name.starts_with('.')
                        || dir_name == "target"
                        || dir_name == "node_modules"
                    {
                        continue;
                    }
                    dirs_to_visit.push((path, depth + 1));
                } else if metadata.is_file() {
                    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                    if re.is_match(filename) {
                        let uri = format!("file://{}", path.to_string_lossy());
                        if !results.iter().any(|(u, _)| u == &uri) {
                            results.push((uri, path));
                        }
                    }
                }
            }
        }
    }

    results
}

fn compile_list_document(query: &str, results: &[(String, PathBuf)]) -> String {
    let mut doc = format!("# Bibliographic Index: {}\n\n", query);
    for (uri, path) in results {
        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("Unknown");
        let metadata = path.metadata().ok();
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let posix_date = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let hash_hex = if let Ok(bytes) = std::fs::read(path) {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        } else {
            "".to_string()
        };

        let mime = guess_mime_type(path);

        doc.push_str("---\n");
        doc.push_str(&format!("Title: {}\n", filename));
        doc.push_str("Author: Local System Proxy\n");
        doc.push_str(&format!("MIME: {}\n", mime));
        doc.push_str(&format!("Size: {}\n", size));
        doc.push_str(&format!("POSIX Date: {}\n", posix_date));
        doc.push_str(&format!("SHA-256: {}\n", hash_hex));
        doc.push_str(&format!("URI: {}\n", uri));
        doc.push_str("---\n");
    }
    doc
}

// BPA Service Implementation

struct BasketService {
    local_eid: OnceCell<Eid>,
    sink: OnceCell<Box<dyn ServiceSink>>,
    config: Config,
    verbose: bool,
    client: reqwest::Client,
    tls_connector: tokio_rustls::TlsConnector,
    active_tasks: Arc<Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    keystore: KeyStore,
    policy: VerifyPolicy,
    sign_key: Option<String>,
    sign_key_file: Option<String>,
    security_source: Option<String>,
}

impl BasketService {
    fn new(
        config: Config,
        verbose: bool,
        keystore: KeyStore,
        policy: VerifyPolicy,
        sign_key: Option<String>,
        sign_key_file: Option<String>,
        security_source: Option<String>,
    ) -> Result<Arc<Self>> {
        let mut client_builder = reqwest::Client::builder();
        if config.insecure_tls {
            client_builder = client_builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
        let client = client_builder.build()?;
        let tls_connector = build_tls_connector(config.insecure_tls)?;

        Ok(Arc::new(Self {
            local_eid: OnceCell::new(),
            sink: OnceCell::new(),
            config,
            verbose,
            client,
            tls_connector,
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            keystore,
            policy,
            sign_key,
            sign_key_file,
            security_source,
        }))
    }

    async fn process_request(self: Arc<Self>, source: Eid, payload: Vec<u8>) -> Result<()> {
        let request = match decode::parse::<BasketRequest>(&payload) {
            Ok(req) => req,
            Err(e) => {
                if self.verbose {
                    eprintln!("Failed to parse payload as BasketRequest: {}", e);
                }
                return Ok(());
            }
        };

        if self.verbose {
            eprintln!("Processing BasketRequest id: {}", request.req_id);
        }

        let reply_dest = match &request.reply_to {
            Some(dest_str) => match dest_str.parse::<Eid>() {
                Ok(eid) => eid,
                Err(_) => source.clone(),
            },
            None => source.clone(),
        };

        let default_lifetime = request.default_lifetime.unwrap_or(3600);

        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        {
            let mut active = self.active_tasks.lock().unwrap();
            active.insert(request.req_id.clone(), cancel_tx);
        }

        let mut item_responses = Vec::new();
        let mut raw_bundles_to_send = Vec::new();

        for (idx, item) in request.items.iter().enumerate() {
            if *cancel_rx.borrow() {
                if self.verbose {
                    eprintln!(
                        "Request {} was cancelled, aborting remaining items.",
                        request.req_id
                    );
                }
                break;
            }

            let item_idx = idx as u64;

            match item.op {
                0 => {
                    match self
                        .handle_get(item, item_idx, default_lifetime, &mut cancel_rx)
                        .await
                    {
                        Ok((item_resp, raw_data_opt)) => {
                            item_responses.push(item_resp);
                            if let Some(raw) = raw_data_opt {
                                raw_bundles_to_send.push(raw);
                            }
                        }
                        Err(e) => {
                            item_responses.push(ItemResponse {
                                item_idx,
                                coap_status: 160,
                                metadata: None,
                                diagnostic: Some(e.to_string()),
                            });
                        }
                    }
                }
                1 => match self.handle_check(item, item_idx, &mut cancel_rx).await {
                    Ok(item_resp) => {
                        item_responses.push(item_resp);
                    }
                    Err(e) => {
                        item_responses.push(ItemResponse {
                            item_idx,
                            coap_status: 160,
                            metadata: None,
                            diagnostic: Some(e.to_string()),
                        });
                    }
                },
                2 => {
                    match self
                        .handle_search(item, item_idx, default_lifetime, &mut cancel_rx)
                        .await
                    {
                        Ok((mut resps, raws)) => {
                            item_responses.append(&mut resps);
                            raw_bundles_to_send.extend(raws);
                        }
                        Err(e) => {
                            item_responses.push(ItemResponse {
                                item_idx,
                                coap_status: 160,
                                metadata: None,
                                diagnostic: Some(e.to_string()),
                            });
                        }
                    }
                }
                3 => {
                    let target_req_id = &item.uri;
                    let success = {
                        let mut active = self.active_tasks.lock().unwrap();
                        if let Some(tx) = active.remove(target_req_id) {
                            let _ = tx.send(true);
                            true
                        } else {
                            false
                        }
                    };

                    let status = if success { 66 } else { 132 };

                    item_responses.push(ItemResponse {
                        item_idx,
                        coap_status: status,
                        metadata: None,
                        diagnostic: Some(format!(
                            "Request {} successfully cancelled",
                            target_req_id
                        )),
                    });
                }
                4 => {
                    match self
                        .handle_list(item, item_idx, default_lifetime, &mut cancel_rx)
                        .await
                    {
                        Ok((item_resp, raw_data_opt)) => {
                            item_responses.push(item_resp);
                            if let Some(raw) = raw_data_opt {
                                raw_bundles_to_send.push(raw);
                            }
                        }
                        Err(e) => {
                            item_responses.push(ItemResponse {
                                item_idx,
                                coap_status: 160,
                                metadata: None,
                                diagnostic: Some(e.to_string()),
                            });
                        }
                    }
                }
                _ => {
                    item_responses.push(ItemResponse {
                        item_idx,
                        coap_status: 132,
                        metadata: None,
                        diagnostic: Some(format!("Unsupported operation: {}", item.op)),
                    });
                }
            }
        }

        {
            let mut active = self.active_tasks.lock().unwrap();
            active.remove(&request.req_id);
        }

        // Send basket-response control bundle
        let response = BasketResponse {
            experiment_tag: request.experiment_tag,
            version: 1,
            req_id: request.req_id.clone(),
            items: item_responses,
        };

        let tagged_resp = Tagged::<44444, _>(&response);
        let (cbor_bytes, _) = encode::emit(&tagged_resp);

        let local_eid = self.local_eid.get().cloned().unwrap_or(Eid::Null);
        let sink = self
            .sink
            .get()
            .ok_or_else(|| anyhow::anyhow!("Sink not registered"))?;

        let (_control_bundle, control_bin) = Builder::new(local_eid.clone(), reply_dest.clone())
            .with_payload(cbor_bytes.into())
            .with_lifetime(std::time::Duration::from_secs(default_lifetime))
            .build(CreationTimestamp::now())
            .map_err(|e| anyhow::anyhow!("Failed to build control bundle: {e}"))?;

        let (_, control_bin_signed) = self.maybe_sign(control_bin.into_vec())?;
        sink.send(Bytes::from(control_bin_signed)).await?;

        // Send raw content bundles
        for (raw_bytes, hash, lifetime) in raw_bundles_to_send {
            let (raw_bundle, raw_bin) = Builder::new(local_eid.clone(), reply_dest.clone())
                .with_payload(raw_bytes.into())
                .with_lifetime(std::time::Duration::from_secs(lifetime))
                .build(CreationTimestamp::now())
                .map_err(|e| anyhow::anyhow!("Failed to build raw content bundle: {e}"))?;

            let (_, raw_bin_signed) = self.maybe_sign(raw_bin.into_vec())?;

            if self.verbose {
                eprintln!(
                    "Sending raw bundle {} with hash {} and size {} bytes",
                    raw_bundle.id.to_key(),
                    hex::encode(&hash),
                    raw_bin_signed.len()
                );
            }
            sink.send(Bytes::from(raw_bin_signed)).await?;
        }

        Ok(())
    }

    fn maybe_sign(&self, binbundle: Vec<u8>) -> Result<(hardy_bpv7::bundle::Bundle, Vec<u8>)> {
        if self.sign_key.is_some() || self.sign_key_file.is_some() {
            let key_mat = dtn_hdy_utils::security::load_key(
                self.sign_key.as_deref(),
                self.sign_key_file.as_deref(),
            )?;
            let sec_source = if let Some(ref sec_str) = self.security_source {
                Some(
                    sec_str
                        .parse::<Eid>()
                        .map_err(|e| anyhow::anyhow!("Invalid security source EID: {e}"))?,
                )
            } else {
                None
            };
            if self.verbose {
                eprintln!("Signing bundle with HMAC-SHA256...");
            }
            let (signed_bundle, signed_binbundle) =
                dtn_hdy_utils::security::sign_bundle(&binbundle, &key_mat, sec_source)?;
            Ok((signed_bundle, signed_binbundle))
        } else {
            let parsed =
                hardy_bpv7::bundle::ParsedBundle::parse(&binbundle, hardy_bpv7::bpsec::no_keys)
                    .map_err(|e| anyhow::anyhow!("Failed to parse bundle: {e}"))?;
            Ok((parsed.bundle, binbundle))
        }
    }

    async fn handle_get(
        &self,
        item: &RequestItem,
        item_idx: u64,
        default_lifetime: u64,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(ItemResponse, Option<(Vec<u8>, Vec<u8>, u64)>)> {
        let max_size = item.max_size.unwrap_or(50 * 1024 * 1024);

        let fetch_fut = fetch_resource(
            &item.uri,
            &self.client,
            &self.tls_connector,
            &self.config,
            max_size,
        );

        let fetch_res = tokio::select! {
            res = fetch_fut => res,
            _ = cancel_rx.changed() => {
                return Err(anyhow::anyhow!("Operation cancelled"));
            }
        };

        let (body, mime) = match fetch_res {
            Ok(res) => res,
            Err(e) => {
                let err_msg = e.to_string();
                let status = if err_msg.contains("Payload too large") {
                    141
                } else if err_msg.contains("Access denied") || err_msg.contains("not allowed") {
                    131
                } else {
                    132
                };

                return Ok((
                    ItemResponse {
                        item_idx,
                        coap_status: status,
                        metadata: None,
                        diagnostic: Some(err_msg),
                    },
                    None,
                ));
            }
        };

        if let Some(formats) = item.accepted_formats.as_ref().filter(|f| !f.is_empty()) {
            let matches = formats
                .iter()
                .any(|f| if f == "*/*" { true } else { mime.contains(f) });
            if !matches {
                return Ok((
                    ItemResponse {
                        item_idx,
                        coap_status: 134,
                        metadata: None,
                        diagnostic: Some(format!("MIME type {} not accepted", mime)),
                    },
                    None,
                ));
            }
        }

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let hash = hasher.finalize().to_vec();

        let last_modified = if let Some(path_str) = item.uri.strip_prefix("file://") {
            std::fs::metadata(path_str)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        } else if let Some(path) = self.config.mappings.get(&item.uri) {
            std::fs::metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        } else {
            None
        };

        let lifetime = item.lifetime_override.unwrap_or(default_lifetime);

        let metadata = ItemMetadata {
            hash: hash.clone(),
            size: Some(body.len() as u64),
            mime_type: Some(mime),
            uri: Some(item.uri.clone()),
            last_modified,
        };

        Ok((
            ItemResponse {
                item_idx,
                coap_status: 69,
                metadata: Some(metadata),
                diagnostic: None,
            },
            Some((body, hash, lifetime)),
        ))
    }

    async fn handle_check(
        &self,
        item: &RequestItem,
        item_idx: u64,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<ItemResponse> {
        let max_size = item.max_size.unwrap_or(50 * 1024 * 1024);

        let fetch_fut = fetch_resource(
            &item.uri,
            &self.client,
            &self.tls_connector,
            &self.config,
            max_size,
        );

        let fetch_res = tokio::select! {
            res = fetch_fut => res,
            _ = cancel_rx.changed() => {
                return Err(anyhow::anyhow!("Operation cancelled"));
            }
        };

        let (body, mime) = match fetch_res {
            Ok(res) => res,
            Err(e) => {
                let err_msg = e.to_string();
                let status = if err_msg.contains("Payload too large") {
                    141
                } else if err_msg.contains("Access denied") || err_msg.contains("not allowed") {
                    131
                } else {
                    132
                };

                return Ok(ItemResponse {
                    item_idx,
                    coap_status: status,
                    metadata: None,
                    diagnostic: Some(err_msg),
                });
            }
        };

        if let Some(formats) = item.accepted_formats.as_ref().filter(|f| !f.is_empty()) {
            let matches = formats
                .iter()
                .any(|f| if f == "*/*" { true } else { mime.contains(f) });
            if !matches {
                return Ok(ItemResponse {
                    item_idx,
                    coap_status: 134,
                    metadata: None,
                    diagnostic: Some(format!("MIME type {} not accepted", mime)),
                });
            }
        }

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let hash = hasher.finalize().to_vec();

        let last_modified = if let Some(path_str) = item.uri.strip_prefix("file://") {
            std::fs::metadata(path_str)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        } else if let Some(path) = self.config.mappings.get(&item.uri) {
            std::fs::metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        } else {
            None
        };

        let metadata = ItemMetadata {
            hash,
            size: Some(body.len() as u64),
            mime_type: Some(mime),
            uri: Some(item.uri.clone()),
            last_modified,
        };

        Ok(ItemResponse {
            item_idx,
            coap_status: 69,
            metadata: Some(metadata),
            diagnostic: None,
        })
    }

    async fn handle_search(
        &self,
        item: &RequestItem,
        item_idx: u64,
        default_lifetime: u64,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(Vec<ItemResponse>, Vec<(Vec<u8>, Vec<u8>, u64)>)> {
        let max_size = item.max_size.unwrap_or(50 * 1024 * 1024);

        let search_results = perform_search(&item.uri, &self.config);

        let mut item_responses = Vec::new();
        let mut raw_bundles_to_send = Vec::new();

        let m = search_results.len();

        if m == 0 {
            item_responses.push(ItemResponse {
                item_idx,
                coap_status: 132,
                metadata: None,
                diagnostic: Some("No matching resources found".to_string()),
            });
            return Ok((item_responses, raw_bundles_to_send));
        }

        if m > 50 {
            if self.verbose {
                eprintln!(
                    "Search found {} matches, exceeding 50. Returning LIST index document.",
                    m
                );
            }
            let markdown_doc = compile_list_document(&item.uri, &search_results);
            let doc_bytes = markdown_doc.into_bytes();

            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&doc_bytes);
            let hash = hasher.finalize().to_vec();

            let lifetime = item.lifetime_override.unwrap_or(default_lifetime);

            let metadata = ItemMetadata {
                hash: hash.clone(),
                size: Some(doc_bytes.len() as u64),
                mime_type: Some("text/markdown; charset=utf-8".to_string()),
                uri: Some(item.uri.clone()),
                last_modified: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                ),
            };

            item_responses.push(ItemResponse {
                item_idx,
                coap_status: 69,
                metadata: Some(metadata),
                diagnostic: Some("Result size exceeded 50, index document returned".to_string()),
            });

            raw_bundles_to_send.push((doc_bytes, hash, lifetime));
            return Ok((item_responses, raw_bundles_to_send));
        }

        for (uri, path) in search_results {
            if *cancel_rx.borrow() {
                break;
            }

            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };

            if bytes.len() as u64 > max_size {
                item_responses.push(ItemResponse {
                    item_idx,
                    coap_status: 141,
                    metadata: None,
                    diagnostic: Some(format!("Resource {} exceeds max size limit", uri)),
                });
                continue;
            }

            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let hash = hasher.finalize().to_vec();

            let mut already_owned = false;
            if let Some(ref have) = item.have_hashes {
                for h in have {
                    if hash.starts_with(h) {
                        already_owned = true;
                        break;
                    }
                }
            }

            let mime = guess_mime_type(&path).to_string();

            let last_modified = path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());

            let metadata = ItemMetadata {
                hash: hash.clone(),
                size: Some(bytes.len() as u64),
                mime_type: Some(mime),
                uri: Some(uri.clone()),
                last_modified,
            };

            item_responses.push(ItemResponse {
                item_idx,
                coap_status: 69,
                metadata: Some(metadata),
                diagnostic: None,
            });

            if !already_owned {
                let lifetime = item.lifetime_override.unwrap_or(default_lifetime);
                raw_bundles_to_send.push((bytes, hash, lifetime));
            }
        }

        Ok((item_responses, raw_bundles_to_send))
    }

    async fn handle_list(
        &self,
        item: &RequestItem,
        item_idx: u64,
        default_lifetime: u64,
        _cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(ItemResponse, Option<(Vec<u8>, Vec<u8>, u64)>)> {
        let search_results = perform_search(&item.uri, &self.config);

        if search_results.is_empty() {
            return Ok((
                ItemResponse {
                    item_idx,
                    coap_status: 132,
                    metadata: None,
                    diagnostic: Some("No matching resources found".to_string()),
                },
                None,
            ));
        }

        let markdown_doc = compile_list_document(&item.uri, &search_results);
        let doc_bytes = markdown_doc.into_bytes();

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&doc_bytes);
        let hash = hasher.finalize().to_vec();

        let lifetime = item.lifetime_override.unwrap_or(default_lifetime);

        let metadata = ItemMetadata {
            hash: hash.clone(),
            size: Some(doc_bytes.len() as u64),
            mime_type: Some("text/markdown; charset=utf-8".to_string()),
            uri: Some(item.uri.clone()),
            last_modified: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
        };

        Ok((
            ItemResponse {
                item_idx,
                coap_status: 69,
                metadata: Some(metadata),
                diagnostic: None,
            },
            Some((doc_bytes, hash, lifetime)),
        ))
    }
}

#[async_trait]
impl BpaService for BasketService {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!(
                "Basket service registered successfully with EID: {}",
                source
            );
        }
        let _ = self.local_eid.set(source.clone());
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {
        if self.verbose {
            eprintln!("Basket service unregistered");
        }
    }

    async fn on_receive(&self, data: Bytes, _expiry: time::OffsetDateTime) {
        let (source, _payload) =
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
                        eprintln!(
                            "Basket request bundle dropped due to strict verification policy."
                        );
                        return;
                    }
                }
                VerifyResult::Unsigned => {
                    if self.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "WARNING: Unsigned basket request bundle received from {}. Dropped due to strict verification policy.",
                            source
                        );
                        return;
                    } else if self.verbose {
                        eprintln!("Received unsigned basket request bundle from {}", source);
                    }
                }
            }
        }

        // Clone Arc self to spawn task
        // But self is passed as Arc inside on_receive?
        // Since BpaService is implemented on Arc<BasketService>, let's make sure it handles Arc reference
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

// Wrapper structure to allow Arc<BasketService> to implement BpaService
struct BasketServiceWrapper(Arc<BasketService>);

#[async_trait]
impl BpaService for BasketServiceWrapper {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        self.0.on_register(source, sink).await;
    }

    async fn on_unregister(&self) {
        self.0.on_unregister().await;
    }

    async fn on_receive(&self, data: Bytes, _expiry: time::OffsetDateTime) {
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

        if self.0.policy != VerifyPolicy::Ignore {
            let res = verify_bundle(&data, &self.0.keystore);
            match res {
                VerifyResult::Valid => {
                    if self.0.verbose {
                        eprintln!("Signature verified successfully for source {}", source);
                    }
                }
                VerifyResult::Invalid(reason) => {
                    eprintln!(
                        "WARNING: Signature verification failed for {}: {}",
                        source, reason
                    );
                    if self.0.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "Basket request bundle dropped due to strict verification policy."
                        );
                        return;
                    }
                }
                VerifyResult::Unsigned => {
                    if self.0.policy == VerifyPolicy::Strict {
                        eprintln!(
                            "WARNING: Unsigned basket request bundle received from {}. Dropped due to strict verification policy.",
                            source
                        );
                        return;
                    } else if self.0.verbose {
                        eprintln!("Received unsigned basket request bundle from {}", source);
                    }
                }
            }
        }

        let service = self.0.clone();
        tokio::spawn(async move {
            if let Err(e) = service.process_request(source, payload).await {
                eprintln!("Error processing request: {}", e);
            }
        });
    }

    async fn on_status_notify(
        &self,
        bundle_id: &hardy_bpv7::bundle::Id,
        from: &Eid,
        kind: hardy_bpa::services::StatusNotify,
        reason: hardy_bpv7::status_report::ReasonCode,
        timestamp: Option<time::OffsetDateTime>,
    ) {
        self.0
            .on_status_notify(bundle_id, from, kind, reason, timestamp)
            .await;
    }
}

fn load_config(explicit_path: Option<PathBuf>, verbose: bool) -> Config {
    let mut config_file = None;
    let mut is_required = false;

    if let Some(path) = explicit_path {
        config_file = Some(path);
        is_required = true;
    } else if let Ok(env_val) = std::env::var("DTNBASKET_CONFIG") {
        config_file = Some(PathBuf::from(env_val));
        is_required = true;
    } else {
        let paths = [
            Some(PathBuf::from("./dtnbasket.toml")),
            directories::ProjectDirs::from("dtn", "", "dtnbasket")
                .map(|dirs| dirs.config_dir().join("dtnbasket.toml")),
            #[cfg(unix)]
            Some(PathBuf::from("/etc/dtn/dtnbasket.toml")),
        ];

        for path in paths.iter().flatten() {
            if path.exists() {
                config_file = Some(path.clone());
                break;
            }
        }
    }

    let builder = ::config::Config::builder();
    let builder = if let Some(ref file) = config_file {
        if verbose {
            eprintln!("Loading config from: {}", file.display());
        }
        builder.add_source(::config::File::from(file.clone()).required(is_required))
    } else {
        if verbose {
            eprintln!("No config file found, using default configuration");
        }
        builder
    };

    let mut config = match builder.build() {
        Ok(config_val) => match config_val.try_deserialize::<Config>() {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Warning: Failed to deserialize config: {e}. Using defaults.");
                Config::default()
            }
        },
        Err(e) => {
            if is_required {
                eprintln!("Error: Failed to load required config file: {e}");
                std::process::exit(1);
            } else {
                Config::default()
            }
        }
    };

    // Resolve relative paths in allowed_dirs
    let parent = config_file.as_ref().and_then(|p| p.parent());
    for dir in &mut config.allowed_dirs {
        if dir.is_relative() {
            if let Some(p) = parent {
                let abs_path = p.join(&dir);
                if abs_path.exists() {
                    *dir = abs_path;
                    continue;
                }
            }
            if let Some(abs_cwd) = std::env::current_dir()
                .ok()
                .map(|cwd| cwd.join(&dir))
                .filter(|p| p.exists())
            {
                *dir = abs_cwd;
            }
        }
    }

    config
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install default rustls crypto provider
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    let args = Args::parse();

    // Resolve port using convention (check env vars first)
    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let port_str = if let Ok(env_port) = std::env::var("HARDY_GRPC_PORT") {
        env_port
    } else if let Ok(env_port) = std::env::var("DTN_WEB_PORT") {
        env_port
    } else if let Some(cli_port) = args.port {
        cli_port.to_string()
    } else {
        // Load config to check its port
        let cfg = load_config(args.config.clone(), args.verbose);
        cfg.bpa_grpc_port.to_string()
    };

    let grpc_addr = format!("http://{}:{}", localhost, port_str);

    if args.verbose {
        eprintln!("Connecting to Hardy BPA at {}", grpc_addr);
    }

    let config = load_config(args.config, args.verbose);

    let mut keystore = KeyStore::load_default_or(args.keystore.as_deref())?;
    if args.verify_key.is_some() || args.verify_key_file.is_some() {
        let key_mat = load_key(args.verify_key.as_deref(), args.verify_key_file.as_deref())?;
        keystore.add_key("*", &key_mat.raw);
    }

    let policy = args.verify_policy;

    let remote_bpa = RemoteBpa::new(grpc_addr);

    let service = BasketService::new(
        config.clone(),
        args.verbose,
        keystore,
        policy,
        args.sign_key,
        args.sign_key_file,
        args.security_source,
    )?;

    let service_name_resolved = args.service.unwrap_or(config.service_name);
    let service_id = if let Ok(num) = service_name_resolved.parse::<u32>() {
        Service::Ipn(num)
    } else {
        Service::Dtn(service_name_resolved.into())
    };

    let wrapper = Arc::new(BasketServiceWrapper(service.clone()));

    let registered_eid = remote_bpa
        .register_service(service_id, wrapper.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Service registration failed: {e}"))?;

    eprintln!("Listening for DTN Basket requests on: {}", registered_eid);

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
    use hardy_cbor::decode::FromCbor;
    use hardy_cbor::encode::{Encoder, Tagged, ToCbor};

    #[test]
    fn test_parse_gemini_url_valid() {
        let url = "gemini://gemini.conman.org/test/path?query=1";
        let (host, port, full) = parse_gemini_url(url).unwrap();
        assert_eq!(host, "gemini.conman.org");
        assert_eq!(port, 1965);
        assert_eq!(full, "gemini://gemini.conman.org/test/path?query=1");
    }

    #[test]
    fn test_parse_gemini_url_custom_port() {
        let url = "gemini://localhost:1966/path";
        let (host, port, full) = parse_gemini_url(url).unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 1966);
        assert_eq!(full, "gemini://localhost:1966/path");
    }

    #[test]
    fn test_check_file_path() {
        let allowed = vec![PathBuf::from("/tmp")];
        // We write a dummy file to ensure canonicalize doesn't error
        let file = tempfile::NamedTempFile::new_in("/tmp").unwrap();
        assert!(check_file_path(file.path(), &allowed));
    }

    #[test]
    fn test_guess_mime_type() {
        assert_eq!(guess_mime_type(Path::new("test.pdf")), "application/pdf");
        assert_eq!(
            guess_mime_type(Path::new("test.md")),
            "text/markdown; charset=utf-8"
        );
        assert_eq!(
            guess_mime_type(Path::new("test.unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_compile_list_document() {
        let temp = tempfile::tempdir().unwrap();
        let file1 = temp.path().join("guide.pdf");
        std::fs::write(&file1, b"pdf content").unwrap();

        let results = vec![("file:///path/guide.pdf".to_string(), file1.clone())];
        let doc = compile_list_document("guide", &results);
        assert!(doc.contains("# Bibliographic Index: guide"));
        assert!(doc.contains("Title: guide.pdf"));
        assert!(doc.contains("MIME: application/pdf"));
    }

    #[test]
    fn test_cbor_basket_request_roundtrip() {
        // Construct a simple request
        let req = BasketRequest {
            experiment_tag: Some(44444),
            version: 1,
            req_id: "req-123".to_string(),
            reply_to: Some("dtn://node2/incoming".to_string()),
            default_lifetime: Some(3600),
            items: vec![RequestItem {
                op: 0,
                uri: "https://example.com/file.bin".to_string(),
                max_size: Some(1024),
                accepted_formats: Some(vec!["application/octet-stream".to_string()]),
                have_hashes: Some(vec![vec![1, 2, 3, 4, 5, 6, 7, 8]]),
                if_modified_since: Some(123456789),
                lifetime_override: Some(600),
            }],
        };

        // Serialize
        let mut encoder = Encoder::new();
        let tagged_req = Tagged::<44444, _>(&req);
        tagged_req.to_cbor(&mut encoder);
        let bytes = encoder.build();

        // Deserialize
        let (parsed_req, _shortest, _len) = BasketRequest::from_cbor(&bytes).unwrap();
        assert_eq!(parsed_req.version, 1);
        assert_eq!(parsed_req.req_id, "req-123");
        assert_eq!(parsed_req.reply_to.unwrap(), "dtn://node2/incoming");
        assert_eq!(parsed_req.default_lifetime.unwrap(), 3600);
        assert_eq!(parsed_req.items.len(), 1);

        let item = &parsed_req.items[0];
        assert_eq!(item.op, 0);
        assert_eq!(item.uri, "https://example.com/file.bin");
        assert_eq!(item.max_size.unwrap(), 1024);
        assert_eq!(
            item.accepted_formats.as_ref().unwrap()[0],
            "application/octet-stream"
        );
        assert_eq!(
            item.have_hashes.as_ref().unwrap()[0],
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(item.if_modified_since.unwrap(), 123456789);
        assert_eq!(item.lifetime_override.unwrap(), 600);
    }

    #[test]
    fn test_config_deserialization() {
        let toml_str = r#"
            service_name = "dtnbasket_test"
            insecure_tls = true
            allowed_dirs = ["/tmp", "/var/log"]
            [mappings]
            "urn:dtn:test" = "/tmp/test.txt"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service_name, "dtnbasket_test");
        assert!(config.insecure_tls);
        assert_eq!(config.allowed_dirs.len(), 2);
        assert_eq!(
            config
                .mappings
                .get("urn:dtn:test")
                .unwrap()
                .to_str()
                .unwrap(),
            "/tmp/test.txt"
        );
    }
}
