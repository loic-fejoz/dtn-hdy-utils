use anyhow::{Context, Result, anyhow};
use hardy_bpv7::bpsec::key::{Key, KeyAlgorithm, KeySource, Operation, Type};
use hardy_bpv7::bpsec::rfc9173::ScopeFlags;
use hardy_bpv7::bpsec::signer::{Context as SignerContext, Signer};
use hardy_bpv7::eid::Eid;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroize;

/// Monotonic counter to ensure collision-free key IDs.
static KEY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Key application direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Outbound: used by this station to sign bundles matching destination pattern
    Outbound,
    /// Inbound: used by this station to verify bundles matching sender/source pattern
    Inbound,
}

/// Verification policy on receiving bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VerifyPolicy {
    /// Accept bundle but mark trust badge in UI (Default)
    #[default]
    Warn,
    /// Reject bundle if signature is invalid
    Strict,
    /// Skip signature verification
    Ignore,
}

/// Cryptographic authentication status of a bundle or message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthStatus {
    /// Signature is present, verified, and valid (Green badge)
    Valid,
    /// Signature verification failed: corrupted data or wrong key (Red badge)
    Invalid,
    /// Bundle is signed, but no corresponding key exists in keystore (Neutral badge)
    MissingKey,
    /// Bundle is unauthenticated: no BPSec BIB block (Neutral badge)
    Unsigned,
}

impl AuthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::MissingKey => "missing_key",
            Self::Unsigned => "unsigned",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "valid" => Self::Valid,
            "invalid" => Self::Invalid,
            "missing_key" => Self::MissingKey,
            _ => Self::Unsigned,
        }
    }
}

/// An entry in the keystore holding raw secret bytes.
#[derive(Clone)]
pub struct KeyEntry {
    pub id: String,
    pub direction: Direction,
    pub pattern: String,
    pub label: Option<String>,
    pub secret: Vec<u8>,
    pub created_at: String,
}

impl Drop for KeyEntry {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

impl std::fmt::Debug for KeyEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEntry")
            .field("id", &self.id)
            .field("direction", &self.direction)
            .field("pattern", &self.pattern)
            .field("label", &self.label)
            .field("secret", &"<redacted>")
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Safe public representation of a key entry (never leaking the secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyInfo {
    pub id: String,
    pub direction: Direction,
    pub pattern: String,
    pub label: Option<String>,
    pub fingerprint: String,
    pub created_at: String,
}

impl KeyEntry {
    pub fn to_info(&self) -> KeyInfo {
        KeyInfo {
            id: self.id.clone(),
            direction: self.direction,
            pattern: self.pattern.clone(),
            label: self.label.clone(),
            fingerprint: compute_fingerprint(&self.secret),
            created_at: self.created_at.clone(),
        }
    }

    pub fn to_hardy_key(&self, operations: &[Operation]) -> Key {
        Key {
            key_type: Type::octet_sequence(self.secret.clone()),
            key_algorithm: Some(KeyAlgorithm::HS256),
            operations: Some(operations.iter().cloned().collect()),
            ..Default::default()
        }
    }
}

/// Computes a domain-separated SHA-256 HMAC fingerprint for display (e.g. "SHA256:4a2f8c19b0de31a7").
/// Domain separation prevents offline dictionary attacks against short passphrases.
pub fn compute_fingerprint(secret: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => {
            // Secret shorter than block size or slice error: hash secret first as fallback
            use sha2::Digest;
            let hashed = Sha256::digest(secret);
            HmacSha256::new_from_slice(&hashed).expect("SHA-256 digest is always 32 bytes")
        }
    };
    mac.update(b"dtn-hdy-ham-bpsec-fingerprint-v1");
    let result = mac.finalize().into_bytes();
    format!("SHA256:{}", hex::encode(&result[..8]))
}

/// File representation for TOML serialization in keystore.toml.
#[derive(Clone, Serialize, Deserialize)]
struct KeyFileEntry {
    pub id: String,
    pub direction: Direction,
    pub pattern: String,
    pub label: Option<String>,
    pub key_hex: String,
    #[serde(default)]
    pub created_at: String,
}

impl Drop for KeyFileEntry {
    fn drop(&mut self) {
        self.key_hex.zeroize();
    }
}

impl std::fmt::Debug for KeyFileEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyFileEntry")
            .field("id", &self.id)
            .field("direction", &self.direction)
            .field("pattern", &self.pattern)
            .field("label", &self.label)
            .field("key_hex", &"<redacted>")
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeystoreConfigFile {
    #[serde(default)]
    pub policy: Option<VerifyPolicy>,
    #[serde(default)]
    pub keys: Vec<KeyFileEntry>,
}

/// Thread-safe Directional Keystore.
#[derive(Debug)]
pub struct DirectionalKeyStore {
    file_path: Option<PathBuf>,
    policy: RwLock<VerifyPolicy>,
    keys: RwLock<Vec<KeyEntry>>,
}

impl DirectionalKeyStore {
    /// Create an empty in-memory keystore with default policy.
    pub fn empty() -> Self {
        Self {
            file_path: None,
            policy: RwLock::new(VerifyPolicy::Warn),
            keys: RwLock::new(Vec::new()),
        }
    }

    /// Load or initialize keystore from given file path.
    pub fn load_or_init(path: &Path) -> Result<Self> {
        let store = Self {
            file_path: Some(path.to_path_buf()),
            policy: RwLock::new(VerifyPolicy::Warn),
            keys: RwLock::new(Vec::new()),
        };

        if path.exists() {
            store.reload_from_file()?;
        } else {
            // Create directory and initial file with 0600 permissions
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            store.persist()?;
        }

        Ok(store)
    }

    pub fn get_policy(&self) -> VerifyPolicy {
        *self.policy.read().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_policy(&self, policy: VerifyPolicy) -> Result<()> {
        *self.policy.write().unwrap_or_else(|p| p.into_inner()) = policy;
        self.persist()
    }

    pub fn list_keys(&self) -> Vec<KeyInfo> {
        self.keys
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|k| k.to_info())
            .collect()
    }

    /// Add or update a key entry and persist.
    /// Enforces:
    /// - Minimum secret length of 16 bytes.
    /// - Uniqueness on `(direction, pattern)` (updates in place if already configured).
    pub fn add_key(
        &self,
        direction: Direction,
        pattern: String,
        secret_bytes: Vec<u8>,
        label: Option<String>,
    ) -> Result<KeyInfo> {
        if secret_bytes.len() < 16 {
            return Err(anyhow!(
                "Secret key must be at least 16 bytes (or 32 hex characters)"
            ));
        }

        let trimmed_pattern = pattern.trim().to_string();
        if trimmed_pattern.is_empty() {
            return Err(anyhow!("Pattern cannot be empty"));
        }

        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "2026-01-01T00:00:00Z".to_string());

        let mut keys = self.keys.write().unwrap_or_else(|p| p.into_inner());

        // Enforce uniqueness on (direction, pattern): update existing if match found
        if let Some(existing) = keys
            .iter_mut()
            .find(|k| k.direction == direction && k.pattern == trimmed_pattern)
        {
            existing.secret = secret_bytes;
            existing.label = label;
            existing.created_at = now;
            let info = existing.to_info();
            drop(keys);
            self.persist()?;
            return Ok(info);
        }

        // New unique key entry
        let counter = KEY_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!(
            "{:x}-{:x}",
            time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
            counter
        );

        let entry = KeyEntry {
            id,
            direction,
            pattern: trimmed_pattern,
            label,
            secret: secret_bytes,
            created_at: now,
        };

        let info = entry.to_info();
        keys.push(entry);
        drop(keys);

        self.persist()?;
        Ok(info)
    }

    /// Remove a key entry by ID.
    pub fn remove_key(&self, id: &str) -> Result<bool> {
        let mut keys = self.keys.write().unwrap_or_else(|p| p.into_inner());
        let initial_len = keys.len();
        keys.retain(|k| k.id != id);
        let removed = keys.len() != initial_len;
        drop(keys);
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Find best matching Outbound key for destination EID (highest specificity tier and score).
    pub fn find_outbound_key(&self, dest_eid: &str) -> Option<KeyEntry> {
        let keys = self.keys.read().unwrap_or_else(|p| p.into_inner());
        let mut candidates: Vec<&KeyEntry> = keys
            .iter()
            .filter(|k| {
                k.direction == Direction::Outbound && matches_eid_pattern(&k.pattern, dest_eid)
            })
            .collect();

        candidates.sort_by_key(|k| std::cmp::Reverse(pattern_specificity(&k.pattern)));
        candidates.first().cloned().cloned()
    }

    /// Find best matching Inbound key for sender/source EID (highest specificity tier and score).
    pub fn find_inbound_key(&self, source_eid: &str) -> Option<KeyEntry> {
        let keys = self.keys.read().unwrap_or_else(|p| p.into_inner());
        let mut candidates: Vec<&KeyEntry> = keys
            .iter()
            .filter(|k| {
                k.direction == Direction::Inbound && matches_eid_pattern(&k.pattern, source_eid)
            })
            .collect();

        candidates.sort_by_key(|k| std::cmp::Reverse(pattern_specificity(&k.pattern)));
        candidates.first().cloned().cloned()
    }

    /// Reload entries from TOML file.
    pub fn reload_from_file(&self) -> Result<()> {
        let path = match &self.file_path {
            Some(p) => p,
            None => return Ok(()),
        };

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read keystore file at {}", path.display()))?;
        let config: KeystoreConfigFile = toml::from_str(&content)
            .with_context(|| format!("Failed to parse keystore TOML at {}", path.display()))?;

        if let Some(pol) = config.policy {
            *self.policy.write().unwrap_or_else(|p| p.into_inner()) = pol;
        }

        let mut loaded = Vec::new();
        for mut file_entry in config.keys {
            let secret = match hex::decode(&file_entry.key_hex) {
                Ok(bytes) => bytes,
                Err(_) => std::mem::take(&mut file_entry.key_hex).into_bytes(),
            };

            loaded.push(KeyEntry {
                id: std::mem::take(&mut file_entry.id),
                direction: file_entry.direction,
                pattern: std::mem::take(&mut file_entry.pattern),
                label: file_entry.label.take(),
                secret,
                created_at: std::mem::take(&mut file_entry.created_at),
            });
        }

        *self.keys.write().unwrap_or_else(|p| p.into_inner()) = loaded;
        Ok(())
    }

    /// Persist entries to TOML file atomically with POSIX 0600 permissions.
    /// Releases in-memory locks before performing disk I/O.
    pub fn persist(&self) -> Result<()> {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        let policy = *self.policy.read().unwrap_or_else(|p| p.into_inner());
        let keys = self.keys.read().unwrap_or_else(|p| p.into_inner());

        let file_entries: Vec<KeyFileEntry> = keys
            .iter()
            .map(|k| KeyFileEntry {
                id: k.id.clone(),
                direction: k.direction,
                pattern: k.pattern.clone(),
                label: k.label.clone(),
                key_hex: hex::encode(&k.secret),
                created_at: k.created_at.clone(),
            })
            .collect();

        drop(keys); // Release read lock before disk I/O

        let config = KeystoreConfigFile {
            policy: Some(policy),
            keys: file_entries,
        };

        let toml_str =
            toml::to_string_pretty(&config).context("Failed to serialize keystore to TOML")?;

        let tmp_path = path.with_extension("tmp");

        {
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;

            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            options.mode(0o600);

            let mut file = options.open(&tmp_path).with_context(|| {
                format!("Failed to open temp keystore at {}", tmp_path.display())
            })?;

            file.write_all(toml_str.as_bytes()).with_context(|| {
                format!("Failed to write temp keystore at {}", tmp_path.display())
            })?;
            file.sync_all().with_context(|| {
                format!("Failed to sync temp keystore at {}", tmp_path.display())
            })?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&tmp_path, perms).with_context(|| {
                format!(
                    "Failed to enforce 0600 permissions on {}",
                    tmp_path.display()
                )
            })?;
        }

        fs::rename(&tmp_path, &path).with_context(|| {
            format!(
                "Failed to atomically rename {} to {}",
                tmp_path.display(),
                path.display()
            )
        })?;

        Ok(())
    }
}

/// Implementation of `hardy_bpv7::bpsec::key::KeySource` for inbound bundle verification.
impl KeySource for DirectionalKeyStore {
    fn key<'a>(&'a self, _source: &Eid, _operations: &[Operation]) -> Option<&'a Key> {
        None
    }
}

/// Parse raw key input string.
/// Decodes as hex only if prefixed by `0x`/`0X` or if exactly 64 hex characters (256 bits).
/// Otherwise treats as a UTF-8 passphrase.
pub fn parse_key_input(input: &str) -> Vec<u8> {
    let trimmed = input.trim();
    if let Some(bytes) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .and_then(|stripped| hex::decode(stripped).ok())
    {
        return bytes;
    }
    if trimmed.len() == 64
        && trimmed.chars().all(|c| c.is_ascii_hexdigit())
        && let Ok(bytes) = hex::decode(trimmed)
    {
        return bytes;
    }
    trimmed.as_bytes().to_vec()
}

/// Computes the hierarchical specificity tier of an EID pattern:
/// - Tier 5: Exact EID match (no wildcards)
/// - Tier 4: Specific node with wildcard service (e.g. `dtn://f5kuv/*`, `ipn:0.1.*`)
/// - Tier 3: Wildcard node with specific service (e.g. `dtn://*/chat`, `dtn://*/serviceb`)
/// - Tier 2: Scheme wildcard (e.g. `dtn://*`, `ipn:*`)
/// - Tier 1: Global wildcard (`*`)
pub fn pattern_tier(pattern: &str) -> u8 {
    let p = pattern.trim();
    if p == "*" {
        return 1;
    }
    if !p.contains('*') {
        return 5;
    }
    if p == "dtn://*" || p == "ipn:*" {
        return 2;
    }
    if let Some(stripped) = p.strip_prefix("dtn://") {
        let parts: Vec<&str> = stripped.split('/').collect();
        if parts.len() >= 2 {
            let node = parts[0];
            let service = parts[1..].join("/");
            if !node.contains('*') && service.contains('*') {
                return 4;
            } else if node.contains('*') && !service.contains('*') {
                return 3;
            }
        }
    } else if let Some(stripped) = p.strip_prefix("ipn:")
        && let Some(pos) = stripped.rfind('.')
    {
        let node = &stripped[..pos];
        let service = &stripped[pos + 1..];
        if !node.contains('*') && service.contains('*') {
            return 4;
        } else if node.contains('*') && !service.contains('*') {
            return 3;
        }
    }
    2
}

/// Calculate pattern specificity: higher tuple (tier, non_wildcard_count) means higher precedence.
pub fn pattern_specificity(pattern: &str) -> (u8, usize) {
    let tier = pattern_tier(pattern);
    let non_wildcard_chars = pattern.chars().filter(|c| *c != '*').count();
    (tier, non_wildcard_chars)
}

/// Pattern matching for DTN and IPN EID expressions (e.g. "dtn://f5kuv/*", "dtn://*/servicea", "dtn://*").
pub fn matches_eid_pattern(pattern: &str, eid: &str) -> bool {
    let p = pattern.trim();
    let s = eid.trim();

    if p == "*" || p == s {
        return true;
    }

    if !p.contains('*') {
        return p == s;
    }

    // Split pattern around '*'
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 2 {
        let (prefix, suffix) = (parts[0], parts[1]);
        return s.starts_with(prefix)
            && s.ends_with(suffix)
            && s.len() >= prefix.len() + suffix.len();
    }

    // General multi-wildcard match
    let mut remainder = s;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !remainder.starts_with(part) {
                return false;
            }
            remainder = &remainder[part.len()..];
        } else if i == parts.len() - 1 {
            return remainder.ends_with(part);
        } else {
            match remainder.find(part) {
                Some(pos) => remainder = &remainder[pos + part.len()..],
                None => return false,
            }
        }
    }
    true
}

/// Sign a bundle with BPSec BIB HMAC-SHA256 using given key entry.
/// Zero-copy / zero redundant parsing: operates directly on the constructed Bundle and its serialized binary representation.
pub fn sign_bundle_with_key(
    bundle: &hardy_bpv7::bundle::Bundle,
    binbundle: &[u8],
    key: &KeyEntry,
    security_source: Option<Eid>,
) -> Result<Vec<u8>> {
    let sec_source = security_source.unwrap_or_else(|| bundle.primary.id.source.clone());
    let hardy_key = key.to_hardy_key(&[Operation::Sign]);

    let signer = Signer::new(bundle, binbundle);
    let signed_signer = signer
        .sign_block(
            1, // payload block target index
            SignerContext::HMAC_SHA2(ScopeFlags::default()),
            sec_source,
            &hardy_key,
        )
        .map_err(|(_, e)| anyhow!("BPSec BIB signing failed: {e}"))?;

    let signed_bytes = signed_signer
        .rebuild()
        .map_err(|e| anyhow!("Failed to rebuild signed bundle: {e}"))?;

    Ok(signed_bytes.into_vec())
}

/// Helper to sign a raw binary bundle by parsing it first (for backwards compatibility).
pub fn sign_raw_bundle(
    binbundle: &[u8],
    key: &KeyEntry,
    security_source: Option<Eid>,
) -> Result<(hardy_bpv7::bundle::Bundle, Vec<u8>)> {
    let parsed_bundle = hardy_bpv7::parse::parse(bytes::Bytes::copy_from_slice(binbundle))
        .map_err(|e| anyhow!("Failed to parse bundle for signing: {e}"))?;

    let signed_bytes =
        sign_bundle_with_key(&parsed_bundle.bundle, binbundle, key, security_source)?;

    let signed_parsed = hardy_bpv7::parse::parse(bytes::Bytes::copy_from_slice(&signed_bytes))
        .map_err(|e| anyhow!("Failed to parse newly signed bundle: {e}"))?;

    Ok((signed_parsed.bundle, signed_bytes))
}

/// Wrapper keystore adapter for Hardy's verify_all_bibs.
struct SingleKeySource<'a> {
    key: &'a Key,
}

impl<'a> KeySource for SingleKeySource<'a> {
    fn key<'k>(&'k self, _source: &Eid, _operations: &[Operation]) -> Option<&'k Key> {
        Some(self.key)
    }
}

/// Verify bundle authentication on a pre-parsed bundle.
/// Enforces:
/// 1. Short-circuit on `VerifyPolicy::Ignore`.
/// 2. Mandatory presence of a BIB block specifically targeting payload block `#1`.
/// 3. Identity concordance: the BIB's `security_source` node must match the bundle's `primary.source` node.
/// 4. HMAC-SHA256 verification against the sender's inbound key.
pub fn verify_parsed_bundle(
    parsed: &hardy_bpv7::parse::Parsed,
    keystore: &DirectionalKeyStore,
) -> AuthStatus {
    if keystore.get_policy() == VerifyPolicy::Ignore {
        return AuthStatus::Unsigned;
    }

    let has_any_bib = !parsed.bibs.is_empty()
        || parsed
            .bundle
            .blocks
            .values()
            .any(|b| matches!(b.block_type, hardy_bpv7::block::Type::BlockIntegrity));

    if !has_any_bib {
        return AuthStatus::Unsigned;
    }

    // Must have a BIB targeting payload block #1
    let Some(payload_bib_op) = parsed
        .bibs
        .values()
        .find(|ops| ops.operations().contains_key(&1))
    else {
        // BIB exists on other blocks but does NOT cover payload block #1
        return AuthStatus::Invalid;
    };

    // Identity concordance: security_source node must match primary.source node
    let bib_source_str = payload_bib_op.source().to_string();
    let primary_source_str = parsed.bundle.primary.id.source.to_string();

    let bib_node = crate::internal_storage::normalize_peer_node(&bib_source_str);
    let primary_node = crate::internal_storage::normalize_peer_node(&primary_source_str);

    if bib_node != primary_node {
        return AuthStatus::Invalid;
    }

    // Look up inbound key for primary source
    let matching_key = match keystore.find_inbound_key(&primary_source_str) {
        Some(k) => k,
        None => return AuthStatus::MissingKey,
    };

    let hardy_key = matching_key.to_hardy_key(&[Operation::Verify]);
    let source_adapter = SingleKeySource { key: &hardy_key };

    let empty_decrypted = std::collections::HashMap::default();
    let empty_updates = std::collections::HashMap::default();

    if hardy_bpv7::checks::verify_all_bibs(
        &parsed.data,
        &source_adapter,
        &parsed.bundle.blocks,
        &parsed.bibs,
        &empty_decrypted,
        &empty_updates,
    )
    .is_err()
    {
        return AuthStatus::Invalid;
    }

    let verifiers_res = hardy_bpv7::checks::begin_payload_verification(
        &parsed.data,
        &source_adapter,
        &parsed.bundle.blocks,
        &parsed.bibs,
    );

    let verifiers = match verifiers_res {
        Ok(v) => v,
        Err(_) => return AuthStatus::Invalid,
    };

    if verifiers.is_empty() {
        return AuthStatus::Invalid;
    }

    let payload_slice = parsed
        .bundle
        .blocks
        .get(&1)
        .and_then(|b| b.payload(&parsed.data))
        .unwrap_or(&[]);

    for (_, verifier) in verifiers.into_iter() {
        let mut v = verifier;
        v.update(payload_slice);
        if v.finish().is_err() {
            return AuthStatus::Invalid;
        }
    }

    AuthStatus::Valid
}

/// Verify bundle authentication from raw bundle bytes using DirectionalKeyStore.
pub fn verify_bundle(binbundle: &[u8], keystore: &DirectionalKeyStore) -> AuthStatus {
    if keystore.get_policy() == VerifyPolicy::Ignore {
        return AuthStatus::Unsigned;
    }

    let parsed = match hardy_bpv7::parse::parse(bytes::Bytes::copy_from_slice(binbundle)) {
        Ok(p) => p,
        Err(_) => return AuthStatus::Unsigned,
    };

    verify_parsed_bundle(&parsed, keystore)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardy_bpv7::builder::Builder;
    use hardy_bpv7::creation_timestamp::CreationTimestamp;

    #[test]
    fn test_pattern_specificity_ranking() {
        let score_exact = pattern_specificity("dtn://f5kuv/serviceb");
        let score_node = pattern_specificity("dtn://f5kuv/*");
        let score_service = pattern_specificity("dtn://*/serviceb");
        let score_long_service = pattern_specificity("dtn://*/very_long_service_name");
        let score_dtn_wildcard = pattern_specificity("dtn://*");
        let score_global = pattern_specificity("*");

        // Tier 5: Exact
        assert_eq!(score_exact.0, 5);
        // Tier 4: Specific node
        assert_eq!(score_node.0, 4);
        // Tier 3: Specific service
        assert_eq!(score_service.0, 3);
        assert_eq!(score_long_service.0, 3);
        // Tier 2: Scheme wildcard
        assert_eq!(score_dtn_wildcard.0, 2);
        // Tier 1: Global wildcard
        assert_eq!(score_global.0, 1);

        // Precedence: Exact > Node > Service > Scheme > Global
        assert!(score_exact > score_node);
        assert!(score_node > score_service);
        // Node MUST beat long service despite character count!
        assert!(score_node > score_long_service);
        assert!(score_service > score_dtn_wildcard);
        assert!(score_dtn_wildcard > score_global);
    }

    #[test]
    fn test_matches_eid_pattern() {
        assert!(matches_eid_pattern("*", "dtn://f5kuv/chat"));
        assert!(matches_eid_pattern("dtn://*", "dtn://f5kuv/chat"));
        assert!(matches_eid_pattern("dtn://f5kuv/*", "dtn://f5kuv/chat"));
        assert!(matches_eid_pattern("dtn://f5kuv/*", "dtn://f5kuv/files"));
        assert!(!matches_eid_pattern("dtn://f5kuv/*", "dtn://f4jxq/chat"));
        assert!(matches_eid_pattern("dtn://*/chat", "dtn://f5kuv/chat"));
        assert!(matches_eid_pattern("dtn://*/chat", "dtn://f4jxq/chat"));
        assert!(!matches_eid_pattern("dtn://*/chat", "dtn://f4jxq/files"));
        assert!(matches_eid_pattern("ipn:0.1.*", "ipn:0.1.42"));
        assert!(!matches_eid_pattern("ipn:0.1.*", "ipn:0.2.42"));
    }

    #[test]
    fn test_keystore_inbound_outbound_separation_and_uniqueness() {
        let temp_dir = tempfile::tempdir().unwrap();
        let keystore_path = temp_dir.path().join("keystore.toml");
        let store = DirectionalKeyStore::load_or_init(&keystore_path).unwrap();

        // Key must be at least 16 bytes
        let short_key = b"short".to_vec();
        assert!(
            store
                .add_key(Direction::Outbound, "dtn://f5kuv/*".into(), short_key, None)
                .is_err()
        );

        let valid_key_out = b"secret_out_f5kuv_16b".to_vec();
        let valid_key_in = b"secret_in_f5kuv_16b_".to_vec();

        // Add outbound key for destination dtn://f5kuv/*
        store
            .add_key(
                Direction::Outbound,
                "dtn://f5kuv/*".to_string(),
                valid_key_out.clone(),
                Some("F5KUV Outbound".to_string()),
            )
            .unwrap();

        // Add inbound key for source dtn://f5kuv/*
        store
            .add_key(
                Direction::Inbound,
                "dtn://f5kuv/*".to_string(),
                valid_key_in.clone(),
                Some("F5KUV Inbound".to_string()),
            )
            .unwrap();

        let out_key = store.find_outbound_key("dtn://f5kuv/chat").unwrap();
        assert_eq!(out_key.secret, valid_key_out);

        let in_key = store.find_inbound_key("dtn://f5kuv/chat").unwrap();
        assert_eq!(in_key.secret, valid_key_in);

        // Verify out_key is not found in inbound direction
        assert!(store.find_inbound_key("dtn://other/chat").is_none());

        // Test uniqueness: updating (Outbound, "dtn://f5kuv/*") replaces existing
        let updated_secret = b"updated_secret_32bytes_outbound_".to_vec();
        store
            .add_key(
                Direction::Outbound,
                "dtn://f5kuv/*".to_string(),
                updated_secret.clone(),
                Some("F5KUV Replaced".to_string()),
            )
            .unwrap();

        assert_eq!(store.list_keys().len(), 2);
        let out_updated = store.find_outbound_key("dtn://f5kuv/chat").unwrap();
        assert_eq!(out_updated.secret, updated_secret);
        assert_eq!(out_updated.label.as_deref(), Some("F5KUV Replaced"));
    }

    #[test]
    fn test_sign_and_verify_bundle_with_identity_concordance() {
        let store = DirectionalKeyStore::empty();
        let key_bytes = b"my_super_secret_ham_key_32_bytes";

        // Add inbound key for source dtn://source/*
        store
            .add_key(
                Direction::Inbound,
                "dtn://source/*".to_string(),
                key_bytes.to_vec(),
                None,
            )
            .unwrap();

        let src_eid: Eid = "dtn://source/test".parse().unwrap();
        let dst_eid: Eid = "dtn://dest/test".parse().unwrap();

        let (unsigned_bundle, unsigned_bin) = Builder::new(src_eid, dst_eid)
            .with_lifetime(std::time::Duration::from_secs(3600))
            .with_payload("Hello BPSec Authenticated Ham!".as_bytes().to_vec().into())
            .build(CreationTimestamp::now())
            .unwrap();

        // Verify before signing: Unsigned
        assert_eq!(verify_bundle(&unsigned_bin, &store), AuthStatus::Unsigned);

        // Sign bundle with matching key
        let key_entry = KeyEntry {
            id: "1".into(),
            direction: Direction::Outbound,
            pattern: "dtn://dest/*".into(),
            label: None,
            secret: key_bytes.to_vec(),
            created_at: "now".into(),
        };

        let signed_bin =
            sign_bundle_with_key(&unsigned_bundle, &unsigned_bin, &key_entry, None).unwrap();

        // Verify after signing: Valid
        assert_eq!(verify_bundle(&signed_bin, &store), AuthStatus::Valid);

        // Simulate identity mismatch: security_source is from dtn://spoofed/ while primary.source is dtn://source/test
        let spoofed_sec_source: Eid = "dtn://spoofed/test".parse().unwrap();
        let spoofed_bin = sign_bundle_with_key(
            &unsigned_bundle,
            &unsigned_bin,
            &key_entry,
            Some(spoofed_sec_source),
        )
        .unwrap();

        // Must reject identity mismatch as Invalid!
        assert_eq!(verify_bundle(&spoofed_bin, &store), AuthStatus::Invalid);

        // Modify payload byte to simulate tampering
        let mut tampered = signed_bin.clone();
        let payload_pos = tampered
            .windows(5)
            .position(|w| w == b"Hello")
            .expect("payload must be in signed bundle");
        tampered[payload_pos] ^= 0x20; // change 'H' to 'h'

        assert_eq!(verify_bundle(&tampered, &store), AuthStatus::Invalid);

        // Verification with an empty keystore (key missing)
        let empty_store = DirectionalKeyStore::empty();
        assert_eq!(
            verify_bundle(&signed_bin, &empty_store),
            AuthStatus::MissingKey
        );

        // Policy Ignore returns Unsigned without verification
        store.set_policy(VerifyPolicy::Ignore).unwrap();
        assert_eq!(verify_bundle(&signed_bin, &store), AuthStatus::Unsigned);
    }
}
