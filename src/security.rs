use anyhow::{Context, Result, anyhow};
use clap::ValueEnum;
use hardy_bpv7::bpsec::key::{Key, KeyAlgorithm, KeySource, Operation, Type};
use hardy_bpv7::bpsec::rfc9173::ScopeFlags;
use hardy_bpv7::bpsec::signer::{Context as SignerContext, Signer};
use hardy_bpv7::bundle::ParsedBundle;
use hardy_bpv7::eid::Eid;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Supported signature algorithms for bundle authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
pub enum SignAlg {
    /// HMAC-SHA256 symmetric MAC
    #[value(name = "hmac-sha256", alias = "hmac")]
    #[serde(rename = "hmac-sha256", alias = "hmac")]
    HmacSha256,
}

/// Verification policy for receiving bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyPolicy {
    /// Drop and reject unauthenticated or invalidly signed bundles.
    Strict,
    /// Log warning to stderr if signature is invalid or missing, but print payload.
    Warn,
    /// Do not perform or check signatures.
    Ignore,
}

/// Container for loaded key material.
#[derive(Debug, Clone)]
pub struct KeyMaterial {
    /// Raw key bytes
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyEntryConfig {
    pub eid: String,
    pub alg: Option<SignAlg>,
    pub key: Option<String>,
    pub key_file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct KeystoreConfig {
    #[serde(default)]
    pub keys: Vec<KeyEntryConfig>,
}

/// A store of public and symmetric keys for verifying received bundles.
#[derive(Debug, Clone, Default)]
pub struct KeyStore {
    entries: Vec<(String, Key)>,
}

impl KeyStore {
    /// Create an empty keystore.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a key entry manually for an EID pattern.
    pub fn add_key(&mut self, eid_pattern: impl Into<String>, key_bytes: &[u8]) {
        let key = Key {
            key_type: Type::OctetSequence {
                key: key_bytes.to_vec().into_boxed_slice(),
            },
            key_algorithm: Some(KeyAlgorithm::HS256),
            operations: Some([Operation::Sign, Operation::Verify].into_iter().collect()),
            ..Default::default()
        };
        self.entries.push((eid_pattern.into(), key));
    }

    /// Load keystore from TOML file.
    pub fn load_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read keystore file at {}", path.display()))?;
        let config: KeystoreConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse keystore TOML at {}", path.display()))?;
        Self::from_config(config, path.parent())
    }

    /// Load keystore from specified path, or default standard path ~/.config/dtn/keystore.toml if present.
    pub fn load_default_or(path_opt: Option<&Path>) -> Result<Self> {
        if let Some(path) = path_opt {
            Self::load_file(path)
        } else if let Some(default_path) = default_keystore_path() {
            if default_path.exists() {
                Self::load_file(&default_path)
            } else {
                Ok(Self::empty())
            }
        } else {
            Ok(Self::empty())
        }
    }

    fn from_config(config: KeystoreConfig, base_dir: Option<&Path>) -> Result<Self> {
        let mut store = Self::empty();
        for entry in config.keys {
            let key_file_str = entry.key_file.as_ref().map(|f| {
                if let Some(dir) = base_dir {
                    dir.join(f).to_string_lossy().to_string()
                } else {
                    f.clone()
                }
            });
            let key_mat = load_key(entry.key.as_deref(), key_file_str.as_deref())?;
            store.add_key(entry.eid, &key_mat.raw);
        }
        Ok(store)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return all candidate keys matching the target EID.
    pub fn find_keys(&self, source: &Eid) -> Vec<&Key> {
        let source_str = source.to_string();
        self.entries
            .iter()
            .filter(|(pattern, _)| matches_eid_pattern(pattern, &source_str))
            .map(|(_, key)| key)
            .collect()
    }
}

pub fn default_keystore_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("dtn").join("keystore.toml"))
}

impl KeySource for KeyStore {
    fn key<'a>(&'a self, source: &Eid, _operations: &[Operation]) -> Option<&'a Key> {
        let source_str = source.to_string();
        for (pattern, key) in &self.entries {
            if matches_eid_pattern(pattern, &source_str) {
                return Some(key);
            }
        }
        None
    }
}

/// Helper function to match EID strings against pattern expressions (supports wildcard '*').
pub fn matches_eid_pattern(pattern: &str, eid: &str) -> bool {
    if pattern == "*" || pattern == eid {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        eid.starts_with(prefix)
    } else {
        false
    }
}

/// Result of bundle verification.
#[derive(Debug, PartialEq, Eq)]
pub enum VerifyResult {
    /// Bundle signature is valid
    Valid,
    /// Bundle signature check failed or key is missing
    Invalid(String),
    /// Bundle has no BPSec signature block
    Unsigned,
}

/// Load key material from either inline key parameter or key file path.
/// Performs auto-detection for PEM, Hex, or raw binary format.
pub fn load_key(key_opt: Option<&str>, key_file_opt: Option<&str>) -> Result<KeyMaterial> {
    let (input_data, is_file) = match (key_opt, key_file_opt) {
        (Some(inline_key), None) => (inline_key.as_bytes().to_vec(), false),
        (None, Some(file_path)) => {
            let data = fs::read(file_path)
                .with_context(|| format!("Failed to read key file at '{file_path}'"))?;
            (data, true)
        }
        (Some(_), Some(_)) => {
            return Err(anyhow!(
                "Cannot specify both inline key material and key file path"
            ));
        }
        (None, None) => {
            return Err(anyhow!(
                "Neither inline key material nor key file path was provided"
            ));
        }
    };

    parse_key_data(&input_data, is_file)
}

fn parse_key_data(data: &[u8], is_file: bool) -> Result<KeyMaterial> {
    let text = String::from_utf8_lossy(data).trim().to_string();

    // 1. PEM Format Detection
    if text.contains("-----BEGIN") {
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let b64_clean = lines.concat().replace(['\r', '\n', ' '], "");
        if let Ok(decoded) = base64_decode(&b64_clean) {
            return Ok(KeyMaterial { raw: decoded });
        }
    }

    // 2. Hex String Detection
    let (is_explicit_hex, hex_candidate) =
        if let Some(stripped) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (true, stripped)
        } else {
            (false, text.as_str())
        };

    if is_explicit_hex {
        let hex_bytes =
            hex::decode(hex_candidate).map_err(|e| anyhow!("Invalid hex key material: {e}"))?;
        return Ok(KeyMaterial { raw: hex_bytes });
    }

    if !hex_candidate.is_empty()
        && hex_candidate.len() % 2 == 0
        && hex_candidate.chars().all(|c| c.is_ascii_hexdigit())
        && let Ok(hex_bytes) = hex::decode(hex_candidate)
    {
        return Ok(KeyMaterial { raw: hex_bytes });
    }

    // 3. Raw Binary / Raw Text Fallback
    if is_file {
        Ok(KeyMaterial { raw: data.to_vec() })
    } else {
        Ok(KeyMaterial {
            raw: text.into_bytes(),
        })
    }
}

fn base64_decode(input: &str) -> Result<Vec<u8>, anyhow::Error> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(input))
        .map_err(|e| anyhow!("Base64 decoding error: {e}"))
}

/// Sign a bundle payload block using RFC 9172 BPSec BIB with the specified key.
pub fn sign_bundle(
    binbundle: &[u8],
    key_mat: &KeyMaterial,
    security_source: Option<Eid>,
) -> Result<(hardy_bpv7::bundle::Bundle, Vec<u8>)> {
    let parsed_bundle = ParsedBundle::parse(binbundle, hardy_bpv7::bpsec::no_keys)
        .map_err(|e| anyhow!("Failed to parse bundle for signing: {e}"))?;

    let sec_source = security_source.unwrap_or_else(|| parsed_bundle.bundle.id.source.clone());

    let key_bytes = key_mat.raw.clone();

    let key = Key {
        key_type: Type::OctetSequence {
            key: key_bytes.into_boxed_slice(),
        },
        key_algorithm: Some(KeyAlgorithm::HS256),
        operations: Some([Operation::Sign].into_iter().collect()),
        ..Default::default()
    };

    let signer = Signer::new(&parsed_bundle.bundle, binbundle);
    let signed_signer = signer
        .sign_block(
            1, // payload block target index
            SignerContext::HMAC_SHA2(ScopeFlags::default()),
            sec_source,
            &key,
        )
        .map_err(|(_, e)| anyhow!("BPSec BIB signing failed: {e}"))?;

    let signed_bytes = signed_signer
        .rebuild()
        .map_err(|e| anyhow!("Failed to rebuild signed bundle: {e}"))?;

    let signed_parsed = ParsedBundle::parse(&signed_bytes, hardy_bpv7::bpsec::no_keys)
        .map_err(|e| anyhow!("Failed to parse newly signed bundle: {e}"))?;

    Ok((signed_parsed.bundle, signed_bytes.into_vec()))
}

struct CapturingKeySource {
    keystore: KeyStore,
    captured_source: Arc<Mutex<Option<Eid>>>,
}

impl KeySource for CapturingKeySource {
    fn key<'b>(&'b self, source: &Eid, operations: &[Operation]) -> Option<&'b Key> {
        if let Ok(mut guard) = self.captured_source.lock() {
            *guard = Some(source.clone());
        }
        self.keystore.key(source, operations)
    }
}

/// Verify bundle authentication using the provided KeyStore.
pub fn verify_bundle(binbundle: &[u8], keystore: &KeyStore) -> VerifyResult {
    let captured_source = Arc::new(Mutex::new(None));
    let captured_clone = Arc::clone(&captured_source);
    let ks = keystore.clone();

    let checked_res = hardy_bpv7::bundle::CheckedBundle::parse(binbundle, move |_, _| {
        Box::new(CapturingKeySource {
            keystore: ks,
            captured_source: captured_clone,
        }) as Box<dyn KeySource>
    });

    match checked_res {
        Ok(checked) => {
            let has_bib = checked
                .bundle
                .blocks
                .values()
                .any(|b| matches!(b.block_type, hardy_bpv7::block::Type::BlockIntegrity));

            if has_bib {
                let sec_source_opt = captured_source
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some(sec_source) = sec_source_opt {
                    let matching_keys = keystore.find_keys(&sec_source);
                    if matching_keys.is_empty() {
                        VerifyResult::Invalid(format!(
                            "Missing key for security source {}",
                            sec_source
                        ))
                    } else {
                        VerifyResult::Valid
                    }
                } else {
                    let sec_source = checked.bundle.id.source.clone();
                    let matching_keys = keystore.find_keys(&sec_source);
                    if matching_keys.is_empty() {
                        VerifyResult::Invalid(format!(
                            "Missing key for security source {}",
                            sec_source
                        ))
                    } else {
                        VerifyResult::Valid
                    }
                }
            } else {
                VerifyResult::Unsigned
            }
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("expecting Array")
                || err_msg.contains("violates RFC 9171 canonical CBOR")
                || err_msg.contains("NotCanonical")
            {
                VerifyResult::Unsigned
            } else {
                VerifyResult::Invalid(err_msg)
            }
        }
    }
}

/// Sign a bundle if key material is supplied.
pub fn maybe_sign_bundle(
    bundle: hardy_bpv7::bundle::Bundle,
    binbundle: Vec<u8>,
    sign_key: Option<&str>,
    sign_key_file: Option<&str>,
    security_source: Option<&str>,
    verbose: bool,
) -> Result<(hardy_bpv7::bundle::Bundle, Vec<u8>)> {
    if sign_key.is_some() || sign_key_file.is_some() {
        let key_mat = load_key(sign_key, sign_key_file)?;
        let sec_source = if let Some(sec_str) = security_source {
            Some(
                crate::normalize_eid(sec_str)
                    .parse::<Eid>()
                    .map_err(|e| anyhow!("Invalid security source EID: {e}"))?,
            )
        } else {
            None
        };
        if verbose {
            eprintln!("Signing bundle with HMAC-SHA256...");
        }
        let (signed_bundle, signed_binbundle) = sign_bundle(&binbundle, &key_mat, sec_source)?;
        Ok((signed_bundle, signed_binbundle))
    } else {
        Ok((bundle, binbundle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardy_bpv7::builder::Builder;
    use hardy_bpv7::creation_timestamp::CreationTimestamp;
    use std::time::Duration;

    #[test]
    fn test_load_key_hmac_and_verify() {
        let secret = "my-secret-hamradio-key";
        let key_mat = load_key(Some(secret), None).unwrap();

        let source = "dtn://node1/service".parse::<Eid>().unwrap();
        let destination = "dtn://node2/service".parse::<Eid>().unwrap();

        let (_orig_bundle, binbundle) = Builder::new(source, destination)
            .with_payload(b"hamradio data".to_vec().into())
            .with_lifetime(Duration::from_secs(3600))
            .build(CreationTimestamp::now())
            .unwrap();

        let (_signed_bundle, signed_bytes) = sign_bundle(&binbundle, &key_mat, None).unwrap();

        // Verify with KeyStore
        let mut keystore = KeyStore::empty();
        keystore.add_key("dtn://node1/*", secret.as_bytes());

        let res = verify_bundle(&signed_bytes, &keystore);
        assert_eq!(res, VerifyResult::Valid);

        // Verify with wrong key
        let mut wrong_keystore = KeyStore::empty();
        wrong_keystore.add_key("dtn://node1/*", b"wrong-secret");
        let res_wrong = verify_bundle(&signed_bytes, &wrong_keystore);
        assert!(matches!(res_wrong, VerifyResult::Invalid(_)));

        // Unsigned bundle verification result
        let res_unsigned = verify_bundle(&binbundle, &keystore);
        assert_eq!(res_unsigned, VerifyResult::Unsigned);
    }

    #[test]
    fn test_verify_signed_missing_key() {
        let secret = "my-secret-hamradio-key";
        let key_mat = load_key(Some(secret), None).unwrap();
        let source = "dtn://node1/service".parse::<Eid>().unwrap();
        let destination = "dtn://node2/service".parse::<Eid>().unwrap();

        let (_orig_bundle, binbundle) = Builder::new(source, destination)
            .with_payload(b"signed payload".to_vec().into())
            .with_lifetime(Duration::from_secs(3600))
            .build(CreationTimestamp::now())
            .unwrap();

        let (_signed_bundle, signed_bytes) = sign_bundle(&binbundle, &key_mat, None).unwrap();

        // Empty keystore -> key is NOT in keystore (Missing Key case)
        let empty_keystore = KeyStore::empty();
        let res = verify_bundle(&signed_bytes, &empty_keystore);
        assert!(matches!(res, VerifyResult::Invalid(_)));
    }

    #[test]
    fn test_eid_pattern_matching() {
        assert!(matches_eid_pattern("dtn://node1/*", "dtn://node1/incoming"));
        assert!(matches_eid_pattern("ipn:1.*", "ipn:1.5"));
        assert!(matches_eid_pattern("*", "dtn://node2/service"));
        assert!(!matches_eid_pattern(
            "dtn://node1/*",
            "dtn://node2/incoming"
        ));
    }
}
