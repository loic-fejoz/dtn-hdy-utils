# ADR 20: BPSec BIB Authentication, Directional Keystore & In-Browser Key Management

## Status
Accepted

## Context
Amateur radio DTN networks (BPv7 - RFC 9171) operate over untrusted, open radio frequency links. In accordance with ITU Radio Regulations Article 25 and national amateur radio licensing rules:
1. **No Encryption:** Secret ciphers or encrypted payloads (such as BPSec Block Confidentiality Blocks / BCB, RFC 9172) are strictly prohibited on amateur radio bands. Transmissions must remain transparent and readable in cleartext.
2. **Sender Authentication & Integrity:** To prevent spoofing, unauthorized relay abuse, malicious bundle tampering, or bogus service commands, RFC 9103 and RFC 9172 provide **Block Integrity Blocks (BIB)** using symmetric HMAC-SHA256 authentication.
3. **Parity with `dtn-android-messenger`:** The companion native mobile application `dtn-android-messenger` displays visual integrity trust badges on received chat messages, files, and bundles (🛡️ Authenticated, ⚠️ Invalid Signature, 🔑 Missing Key, ❓ Unsigned).
4. **Directional & Per-Service Keying:** In real-world radio networks, keys must not be conflated between transmission directions ($A \rightarrow B \neq B \rightarrow A$). Moreover, operators need the ability to assign a global fallback key as well as granular keys targeting specific services (e.g. `dtn://*/chat`, `dtn://f5kuv/*`, `dtn://f5kuv/serviceb`, `ipn:0.1.*`).

## Decision

We introduce a comprehensive, direction-separated BPSec BIB authentication framework and in-browser Keystore for `dtn-hdy-ham`:

### 1. Strict Directional Key Separation & Uniqueness
Keys are partitioned into two distinct sets with uniqueness enforced on `(direction, pattern)`:
- **Outbound Keys (Signing):** Indexed by **destination EID pattern** (e.g., `dtn://*/chat`, `dtn://f5kuv/*`, `dtn://*`, `*`). When injecting a bundle, the station automatically selects the most specific matching outbound key to compute the HMAC-SHA256 BIB block targeting payload block `#1`. If signing fails when an outbound key matches, injection fails closed.
- **Inbound Keys (Verification):** Indexed by **sender EID pattern** (e.g., `dtn://f5kuv/*`, `dtn://*`). When receiving a bundle carrying a BIB block, the station verifies that the BIB's `security_source` matches the bundle's `primary.source` node identity, covers payload block `#1`, and verifies the HMAC with the matching inbound key.

### 2. Deterministic Hierarchical Pattern Specificity
EID patterns support both `dtn://` and `ipn:` schemes with wildcard `*`. When multiple keys match, precedence follows a strict deterministic hierarchy (independent of service or node name character length):
1. **Exact Match:** `dtn://f5kuv/serviceb`
2. **Specific Node + Wildcard Service:** `dtn://f5kuv/*`, `ipn:0.1.*`
3. **Wildcard Node + Specific Service:** `dtn://*/serviceb`
4. **Scheme Wildcard:** `dtn://*`, `ipn:*`
5. **Global Wildcard:** `*`

### 3. Verification Policy & Visual Trust Badges
- **Global Policy (`VerifyPolicy`):**
  - `Warn` (Default): Accept and store bundles regardless of verification outcome, attaching explicit visual trust badges.
  - `Strict`: Immediately reject and drop bundles carrying invalid signatures (`AuthStatus::Invalid`) at ingestion. Unsigned bundles (`Unsigned`) and bundles signed by a peer without a configured inbound key (`MissingKey`) are accepted.
  - `Ignore`: Short-circuit and skip cryptographic verification.
- **Visual Trust Badges across Chat, Files, and Bundles:**
  - 🛡️ **Valid / Authenticated (Green):** Valid HMAC-SHA256 signature matching the sender key and covering payload block `#1`.
  - ⚠️ **Invalid Signature (Red):** Altered payload, mismatched secret key, or `security_source` / `primary.source` identity mismatch.
  - ❓ **Unsigned / Missing Key (Neutral):** Unauthenticated DTN BPv7 bundle, or bundle signed by an unknown sender for which no inbound key is configured (both share the same neutral `❓` icon).

### 4. Keystore Storage & Cybersecurity Best Practices
- **Atomic File Security (`keystore.toml`):** Created and persisted atomically (via temporary file + rename) with strict POSIX `0600` permissions applied at file creation (`OpenOptionsExt::mode(0o600)`) to eliminate TOCTOU exposure.
- **Zero Plaintext Leakage via API, Logs & Memory:** `GET /api/v1/security/keys` never returns raw secrets. It returns metadata: `id`, `direction`, `pattern`, `label`, a domain-separated `fingerprint` (`HMAC-SHA256(secret, "dtn-hdy-ham-bpsec-fingerprint-v1")` truncated to 8 bytes), and creation timestamp. `Debug` implementations redact secrets (`<redacted>`) and secret buffers are zeroized on drop.
- **Timing Attack Mitigation:** Signature comparisons are performed using constant-time comparison algorithms.
- **Key Entry Disambiguation & Minimum Strength:** Requires at least 16 bytes of key material. Supports UTF-8 passphrases ($\ge 16$ bytes) and explicit hexadecimal keys (`0x`-prefixed or 64-character 256-bit hex), as well as an in-browser cryptographically secure random key generator (`crypto.getRandomValues`).

### 5. UI Integration in Configuration View
- Integrated into [`ConfigView.tsx`](../dtn-hdy-ham/web/src/components/ConfigView.tsx) to avoid top-level navbar clutter.
- Provides a dedicated *Security & BPSec Keys* card with separate tables for Outbound and Inbound keys, policy switcher, and a key addition modal.

## Consequences

### Positive
- Fully aligns `dtn-hdy-ham` with `dtn-android-messenger` security trust badges.
- Guarantees strict compliance with amateur radio regulations (cleartext data with cryptographic authentication).
- Prevents reflection and spoofing attacks through directional key separation.
- Zero-friction automated signing for operators once keys are configured.

### Negative
- Computing HMAC-SHA256 over large file bundles introduces a minor CPU overhead during ingestion, mitigated by streaming hashing and efficient in-memory buffers.
