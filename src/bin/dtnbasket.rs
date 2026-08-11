use anyhow::Result;
use bytes::Bytes;
use clap::Parser;
use dtn_hdy_utils::resolve_grpc_port;
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
    #[serde(default = "default_search_depth")]
    max_search_depth: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            insecure_tls: false,
            bpa_grpc_port: 50051,
            service_name: "dtnbasket".to_string(),
            allowed_dirs: Vec::new(),
            mappings: HashMap::new(),
            max_search_depth: 3,
        }
    }
}

fn default_grpc_port() -> u16 {
    50051
}

fn default_service_name() -> String {
    "dtnbasket".to_string()
}

fn default_search_depth() -> u32 {
    3
}

use dtn_hdy_utils::basket::*;

const SEARCH_RESULT_INLINE_LIMIT: usize = 50;

// Client fetching logic and security checks

fn build_tls_connector(insecure_tls: bool) -> Result<tokio_rustls::TlsConnector> {
    let client_config = if insecure_tls {
        let mut config = ClientConfig::builder()
            .with_root_certificates(RootCertStore::empty())
            .with_no_client_auth();
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(SelfSignedVerifier));
        config
    } else {
        let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        ClientConfig::builder()
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
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    tls_stream.write_all(request.as_bytes()).await?;
    tls_stream.flush().await?;

    let mut reader = BufReader::new(tls_stream);

    // Read response header up to \n
    let mut header_bytes = Vec::new();
    let n = reader.read_until(b'\n', &mut header_bytes).await?;
    if n == 0 {
        return Err(anyhow::anyhow!("Connection closed before header received"));
    }
    if header_bytes.len() > 1026 {
        return Err(anyhow::anyhow!("Response header too long"));
    }

    let header_str = String::from_utf8_lossy(&header_bytes);
    let header_str = header_str.trim_end_matches(&['\r', '\n'][..]);
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
                let n = reader.read(&mut chunk).await?;
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
        if path.starts_with(dir) {
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
        Some("json") => "application/json",
        Some("toml") => "application/toml",
        Some("csv") => "text/csv; charset=utf-8",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

fn is_safe_ip(ip: std::net::IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return false;
    }
    match ip {
        std::net::IpAddr::V4(v4) => !v4.is_private() && !v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            // Check if it's an IPv4-mapped IPv6 address
            if let Some(v4) = v6.to_ipv4() {
                return !v4.is_loopback()
                    && !v4.is_unspecified()
                    && !v4.is_private()
                    && !v4.is_link_local();
            }
            let is_link_local = (v6.segments()[0] & 0xffc0) == 0xfe80;
            let is_unique_local = (v6.segments()[0] & 0xfe00) == 0xfc00;
            !is_link_local && !is_unique_local
        }
    }
}

async fn is_safe_host(host: &str, port: u16) -> bool {
    let host_trimmed = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host_trimmed.parse::<std::net::IpAddr>() {
        return is_safe_ip(ip);
    }

    let host_lower = host.to_lowercase();
    if host_lower == "localhost" {
        return false;
    }

    // Resolve domain using tokio::net::lookup_host
    if let Ok(addrs) = tokio::net::lookup_host(format!("{}:{}", host, port)).await {
        let mut count = 0;
        for addr in addrs {
            count += 1;
            if !is_safe_ip(addr.ip()) {
                return false;
            }
        }
        count > 0
    } else {
        false
    }
}

#[derive(Debug)]
enum FetchError {
    PayloadTooLarge,
    Forbidden(String),
    NotFound(String),
    Other(anyhow::Error),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge => write!(f, "Payload too large"),
            Self::Forbidden(msg) => write!(f, "Access denied: {}", msg),
            Self::NotFound(msg) => write!(f, "Not found: {}", msg),
            Self::Other(err) => write!(f, "{}", err),
        }
    }
}

impl std::error::Error for FetchError {}

async fn fetch_resource(
    uri: &str,
    client: &reqwest::Client,
    connector: &tokio_rustls::TlsConnector,
    config: &Config,
    max_size: u64,
) -> Result<(Vec<u8>, String), FetchError> {
    let mut current_uri = uri.to_string();
    let mut redirect_count = 0;

    loop {
        if current_uri.starts_with("http://") || current_uri.starts_with("https://") {
            let url = reqwest::Url::parse(&current_uri).map_err(|e| FetchError::Other(e.into()))?;
            let port = url
                .port()
                .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
            let host_safe = match url.host_str() {
                Some(host) => is_safe_host(host, port).await,
                None => true,
            };
            if !host_safe {
                return Err(FetchError::Forbidden(format!(
                    "Forbidden host (SSRF protection): {}",
                    url.host_str().unwrap_or("")
                )));
            }

            let res = client
                .get(&current_uri)
                .send()
                .await
                .map_err(|e| FetchError::NotFound(e.to_string()))?;
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
                    return Err(FetchError::PayloadTooLarge);
                }

                let bytes = res.bytes().await.map_err(|e| FetchError::Other(e.into()))?;
                if bytes.len() as u64 > max_size {
                    return Err(FetchError::PayloadTooLarge);
                }
                return Ok((bytes.to_vec(), mime));
            } else if status.is_redirection() {
                if redirect_count >= 5 {
                    return Err(FetchError::Other(anyhow::anyhow!("Too many redirects")));
                }
                redirect_count += 1;
                let location = res
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .ok_or_else(|| {
                        FetchError::Other(anyhow::anyhow!("Redirect without Location header"))
                    })?;

                let base =
                    reqwest::Url::parse(&current_uri).map_err(|e| FetchError::Other(e.into()))?;
                let resolved = base
                    .join(location)
                    .map_err(|e| FetchError::Other(e.into()))?;

                let scheme = resolved.scheme();
                if scheme != "http" && scheme != "https" && scheme != "gemini" {
                    return Err(FetchError::Forbidden(format!(
                        "Forbidden redirect scheme: {}",
                        scheme
                    )));
                }

                let port = resolved.port().unwrap_or(if resolved.scheme() == "https" {
                    443
                } else {
                    80
                });
                let host_safe = match resolved.host_str() {
                    Some(host) => is_safe_host(host, port).await,
                    None => true,
                };
                if !host_safe {
                    return Err(FetchError::Forbidden(format!(
                        "Forbidden redirect host (SSRF protection): {}",
                        resolved.host_str().unwrap_or("")
                    )));
                }

                current_uri = resolved.to_string();
                continue;
            } else {
                return Err(FetchError::NotFound(format!("HTTP error: {}", status)));
            }
        } else if current_uri.starts_with("gemini://") {
            let (host, port, _) = parse_gemini_url(&current_uri).map_err(FetchError::Other)?;
            if !is_safe_host(&host, port).await {
                return Err(FetchError::Forbidden(format!(
                    "Forbidden host (SSRF protection): {}",
                    host
                )));
            }

            match fetch_gemini(&current_uri, connector, max_size).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    let err_msg = e.to_string();
                    if let Some(redirect_meta) = err_msg.strip_prefix("REDIRECT: ") {
                        if redirect_count >= 5 {
                            return Err(FetchError::Other(anyhow::anyhow!("Too many redirects")));
                        }
                        redirect_count += 1;
                        let base = reqwest::Url::parse(&current_uri)
                            .map_err(|e| FetchError::Other(e.into()))?;
                        let resolved = base
                            .join(redirect_meta.trim())
                            .map_err(|e| FetchError::Other(e.into()))?;

                        let scheme = resolved.scheme();
                        if scheme != "http" && scheme != "https" && scheme != "gemini" {
                            return Err(FetchError::Forbidden(format!(
                                "Forbidden redirect scheme: {}",
                                scheme
                            )));
                        }

                        let port = resolved.port().unwrap_or(1965);
                        let host_safe = match resolved.host_str() {
                            Some(host) => is_safe_host(host, port).await,
                            None => true,
                        };
                        if !host_safe {
                            return Err(FetchError::Forbidden(format!(
                                "Forbidden redirect host (SSRF protection): {}",
                                resolved.host_str().unwrap_or("")
                            )));
                        }

                        current_uri = resolved.to_string();
                        continue;
                    } else if err_msg.contains("Payload too large") {
                        return Err(FetchError::PayloadTooLarge);
                    } else {
                        return Err(FetchError::NotFound(err_msg));
                    }
                }
            }
        } else if let Some(file_path_str) = current_uri.strip_prefix("file://") {
            let path = PathBuf::from(file_path_str);
            if !check_file_path(&path, &config.allowed_dirs) {
                return Err(FetchError::Forbidden(format!(
                    "Access denied or file not allowed: {}",
                    file_path_str
                )));
            }
            let bytes = std::fs::read(&path).map_err(|e| FetchError::NotFound(e.to_string()))?;
            if bytes.len() as u64 > max_size {
                return Err(FetchError::PayloadTooLarge);
            }
            let mime = guess_mime_type(&path).to_string();
            return Ok((bytes, mime));
        } else if let Some(mapped_path) = config.mappings.get(&current_uri) {
            let canonical = mapped_path
                .canonicalize()
                .map_err(|e| FetchError::NotFound(e.to_string()))?;
            let bytes =
                std::fs::read(&canonical).map_err(|e| FetchError::NotFound(e.to_string()))?;
            if bytes.len() as u64 > max_size {
                return Err(FetchError::PayloadTooLarge);
            }
            let mime = guess_mime_type(&canonical).to_string();
            return Ok((bytes, mime));
        } else {
            return Err(FetchError::NotFound(format!(
                "Unsupported URI scheme or mapping: {}",
                current_uri
            )));
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
            if depth > config.max_search_depth {
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

        doc.push_str(&format!("### {}\n\n", filename));
        doc.push_str(&format!("- **Title**: {}\n", filename));
        doc.push_str("- **Author**: Local System Proxy\n");
        doc.push_str(&format!("- **MIME**: {}\n", mime));
        doc.push_str(&format!("- **Size**: {} bytes\n", size));
        doc.push_str(&format!("- **POSIX Date**: {}\n", posix_date));
        doc.push_str(&format!("- **SHA-256**: {}\n", hash_hex));
        doc.push_str(&format!("- **URI**: {}\n\n", uri));
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
        let mut client_builder =
            reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
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
                                coap_status: coap_status::INTERNAL_SERVER_ERROR,
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
                            coap_status: coap_status::INTERNAL_SERVER_ERROR,
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
                                coap_status: coap_status::INTERNAL_SERVER_ERROR,
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

                    let status = if success {
                        coap_status::CHANGED
                    } else {
                        coap_status::NOT_FOUND
                    };

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
                                coap_status: coap_status::INTERNAL_SERVER_ERROR,
                                metadata: None,
                                diagnostic: Some(e.to_string()),
                            });
                        }
                    }
                }
                _ => {
                    item_responses.push(ItemResponse {
                        item_idx,
                        coap_status: coap_status::NOT_FOUND,
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

        let (control_bundle, control_bin) = Builder::new(local_eid.clone(), reply_dest.clone())
            .with_payload(cbor_bytes.into())
            .with_lifetime(std::time::Duration::from_secs(default_lifetime))
            .build(CreationTimestamp::now())
            .map_err(|e| anyhow::anyhow!("Failed to build control bundle: {e}"))?;

        let (_, control_bin_signed) = self.maybe_sign(&control_bundle, control_bin.into_vec())?;
        sink.send(Bytes::from(control_bin_signed)).await?;

        // Send raw content bundles
        for (raw_bytes, hash, lifetime) in raw_bundles_to_send {
            let (raw_bundle, raw_bin) = Builder::new(local_eid.clone(), reply_dest.clone())
                .with_payload(raw_bytes.into())
                .with_lifetime(std::time::Duration::from_secs(lifetime))
                .build(CreationTimestamp::now())
                .map_err(|e| anyhow::anyhow!("Failed to build raw content bundle: {e}"))?;

            let (_, raw_bin_signed) = self.maybe_sign(&raw_bundle, raw_bin.into_vec())?;

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

    fn maybe_sign(
        &self,
        bundle: &hardy_bpv7::bundle::Bundle,
        binbundle: Vec<u8>,
    ) -> Result<(hardy_bpv7::bundle::Bundle, Vec<u8>)> {
        dtn_hdy_utils::security::maybe_sign_bundle(
            bundle.clone(),
            binbundle,
            self.sign_key.as_deref(),
            self.sign_key_file.as_deref(),
            self.security_source.as_deref(),
            self.verbose,
        )
    }

    async fn fetch_and_hash_item(
        &self,
        item: &RequestItem,
        item_idx: u64,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(ItemResponse, Option<(Vec<u8>, Vec<u8>)>)> {
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
                let status = match e {
                    FetchError::PayloadTooLarge => coap_status::REQUEST_ENTITY_TOO_LARGE,
                    FetchError::Forbidden(_) => coap_status::FORBIDDEN,
                    FetchError::NotFound(_) => coap_status::NOT_FOUND,
                    FetchError::Other(_) => coap_status::NOT_FOUND,
                };

                return Ok((
                    ItemResponse {
                        item_idx,
                        coap_status: status,
                        metadata: None,
                        diagnostic: Some(e.to_string()),
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
                        coap_status: coap_status::NOT_ACCEPTABLE,
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
                coap_status: coap_status::CONTENT,
                metadata: Some(metadata),
                diagnostic: None,
            },
            Some((body, hash)),
        ))
    }

    async fn handle_get(
        &self,
        item: &RequestItem,
        item_idx: u64,
        default_lifetime: u64,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(ItemResponse, Option<(Vec<u8>, Vec<u8>, u64)>)> {
        let (item_resp, body_hash_opt) =
            self.fetch_and_hash_item(item, item_idx, cancel_rx).await?;
        let lifetime = item.lifetime_override.unwrap_or(default_lifetime);
        let bundle_to_send = body_hash_opt.map(|(body, hash)| (body, hash, lifetime));
        Ok((item_resp, bundle_to_send))
    }

    async fn handle_check(
        &self,
        item: &RequestItem,
        item_idx: u64,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<ItemResponse> {
        let (item_resp, _) = self.fetch_and_hash_item(item, item_idx, cancel_rx).await?;
        Ok(item_resp)
    }

    #[allow(clippy::type_complexity)]
    async fn handle_search(
        &self,
        item: &RequestItem,
        item_idx: u64,
        default_lifetime: u64,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(Vec<ItemResponse>, Vec<(Vec<u8>, Vec<u8>, u64)>)> {
        let max_size = item.max_size.unwrap_or(50 * 1024 * 1024);

        let query_clone = item.uri.clone();
        let config_clone = self.config.clone();
        let search_results =
            tokio::task::spawn_blocking(move || perform_search(&query_clone, &config_clone))
                .await?;

        let mut item_responses = Vec::new();
        let mut raw_bundles_to_send = Vec::new();

        let m = search_results.len();

        if m == 0 {
            item_responses.push(ItemResponse {
                item_idx,
                coap_status: coap_status::NOT_FOUND,
                metadata: None,
                diagnostic: Some("No matching resources found".to_string()),
            });
            return Ok((item_responses, raw_bundles_to_send));
        }

        if m > SEARCH_RESULT_INLINE_LIMIT {
            if self.verbose {
                eprintln!(
                    "Search found {} matches, exceeding {}. Returning LIST index document.",
                    m, SEARCH_RESULT_INLINE_LIMIT
                );
            }
            let query_clone = item.uri.clone();
            let results_clone = search_results.clone();
            let markdown_doc = tokio::task::spawn_blocking(move || {
                compile_list_document(&query_clone, &results_clone)
            })
            .await?;
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
                coap_status: coap_status::CONTENT,
                metadata: Some(metadata),
                diagnostic: Some(format!(
                    "Result size exceeded {}, index document returned",
                    SEARCH_RESULT_INLINE_LIMIT
                )),
            });

            raw_bundles_to_send.push((doc_bytes, hash, lifetime));
            return Ok((item_responses, raw_bundles_to_send));
        }

        let have_hashes_clone = item.have_hashes.clone();
        let lifetime_override = item.lifetime_override;
        let cancel_rx_clone = cancel_rx.clone();

        let (mut inline_resps, inline_raws) = tokio::task::spawn_blocking(
            move || -> Result<(Vec<ItemResponse>, Vec<(Vec<u8>, Vec<u8>, u64)>)> {
                let mut item_responses = Vec::new();
                let mut raw_bundles_to_send = Vec::new();

                for (uri, path) in search_results {
                    if *cancel_rx_clone.borrow() {
                        break;
                    }

                    let bytes = match std::fs::read(&path) {
                        Ok(b) => b,
                        Err(_) => continue,
                    };

                    if bytes.len() as u64 > max_size {
                        item_responses.push(ItemResponse {
                            item_idx,
                            coap_status: coap_status::REQUEST_ENTITY_TOO_LARGE,
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
                    if let Some(ref have) = have_hashes_clone {
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
                        coap_status: coap_status::CONTENT,
                        metadata: Some(metadata),
                        diagnostic: None,
                    });

                    if !already_owned {
                        let lifetime = lifetime_override.unwrap_or(default_lifetime);
                        raw_bundles_to_send.push((bytes, hash, lifetime));
                    }
                }

                Ok((item_responses, raw_bundles_to_send))
            },
        )
        .await??;

        item_responses.append(&mut inline_resps);
        raw_bundles_to_send.extend(inline_raws);

        Ok((item_responses, raw_bundles_to_send))
    }

    async fn handle_list(
        &self,
        item: &RequestItem,
        item_idx: u64,
        default_lifetime: u64,
        _cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(ItemResponse, Option<(Vec<u8>, Vec<u8>, u64)>)> {
        let query_clone = item.uri.clone();
        let config_clone = self.config.clone();
        let search_results =
            tokio::task::spawn_blocking(move || perform_search(&query_clone, &config_clone))
                .await?;

        if search_results.is_empty() {
            return Ok((
                ItemResponse {
                    item_idx,
                    coap_status: coap_status::NOT_FOUND,
                    metadata: None,
                    diagnostic: Some("No matching resources found".to_string()),
                },
                None,
            ));
        }

        let query_clone = item.uri.clone();
        let results_clone = search_results.clone();
        let markdown_doc = tokio::task::spawn_blocking(move || {
            compile_list_document(&query_clone, &results_clone)
        })
        .await?;
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
                coap_status: coap_status::CONTENT,
                metadata: Some(metadata),
                diagnostic: None,
            },
            Some((doc_bytes, hash, lifetime)),
        ))
    }

    pub async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        if self.verbose {
            eprintln!(
                "Basket service registered successfully with EID: {}",
                source
            );
        }
        let _ = self.local_eid.set(source.clone());
        let _ = self.sink.set(sink);
    }

    pub async fn on_unregister(&self) {
        eprintln!("Error: Basket service unregistered (connection lost). Exiting.");
        std::process::exit(1);
    }

    pub async fn on_status_notify(
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

    // Resolve relative paths in allowed_dirs and canonicalize all allowed_dirs
    let parent = config_file.as_ref().and_then(|p| p.parent());
    let mut canonical_dirs = Vec::new();
    for dir in &config.allowed_dirs {
        let abs_path = if dir.is_relative() {
            let mut resolved = None;
            if let Some(p) = parent {
                let p_join = p.join(dir);
                if p_join.exists() {
                    resolved = Some(p_join);
                }
            }
            if resolved.is_none() {
                resolved = std::env::current_dir()
                    .ok()
                    .map(|cwd| cwd.join(dir))
                    .filter(|p| p.exists());
            }
            resolved.unwrap_or_else(|| dir.clone())
        } else {
            dir.clone()
        };
        if let Ok(canon) = abs_path.canonicalize() {
            canonical_dirs.push(canon);
        } else {
            canonical_dirs.push(abs_path);
        }
    }
    config.allowed_dirs = canonical_dirs;

    config
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install default rustls crypto provider
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    let args = Args::parse();

    let config = load_config(args.config, args.verbose);

    // Resolve port using convention (check env vars first)
    let localhost = if args.ipv6 { "[::1]" } else { "127.0.0.1" };
    let port_str = resolve_grpc_port(args.port.or(Some(config.bpa_grpc_port)));

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
        assert!(doc.contains("Title**: guide.pdf"));
        assert!(doc.contains("MIME**: application/pdf"));
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

    #[tokio::test]
    async fn test_is_safe_ip_and_host() {
        assert!(!is_safe_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_safe_ip("0.0.0.0".parse().unwrap()));
        assert!(!is_safe_ip("192.168.1.1".parse().unwrap()));
        assert!(!is_safe_ip("10.0.0.1".parse().unwrap()));
        assert!(!is_safe_ip("::1".parse().unwrap()));
        assert!(!is_safe_ip("::".parse().unwrap()));
        assert!(!is_safe_ip("fe80::1".parse().unwrap()));
        assert!(!is_safe_ip("fc00::1".parse().unwrap()));

        assert!(!is_safe_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_safe_ip("::ffff:192.168.1.1".parse().unwrap()));

        assert!(is_safe_ip("8.8.8.8".parse().unwrap()));
        assert!(is_safe_ip("2001:4860:4860::8888".parse().unwrap()));

        assert!(!is_safe_host("127.0.0.1", 80).await);
        assert!(!is_safe_host("[::ffff:127.0.0.1]", 80).await);
        assert!(is_safe_host("8.8.8.8", 80).await);

        assert!(!is_safe_host("localhost", 80).await);
    }
}
