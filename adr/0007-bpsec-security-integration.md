# ADR 7: BPSec Security Integration and Service API Migration

## Status
Accepted

## Context
The `dtn-hdy-utils` utilities initially used the high-level `Application` trait from `hardy-bpa` for receiving bundles. This API provides decoded payload ADUs (Application Data Units) — the raw bundle bytes are fully processed by the BPA before delivery. As a consequence, it was impossible to access and verify BPSec Block Integrity Block (BIB) signatures on received bundles at the application level.

To support authenticated DTN communication (required for ham radio and similar constrained environments), we needed:
1. Cryptographic authentication of incoming bundles using BPSec BIB (RFC 9172).
2. A configurable verification policy (`strict`, `warn`, `ignore`) so operators can choose the enforcement level.
3. Optional signing of outgoing bundles using a symmetric key.

Additionally, the original implementation had explored both HMAC-SHA256 and Ed25519 asymmetric signatures. RFC 9172/9173 does not yet standardize an asymmetric BPSec context, and the `hardy-bpv7` crate does not implement one. Ed25519 support was therefore removed to avoid non-standard behavior.

## Decision

### 1. Migrate all receiving utilities to the low-level `BpaService` trait

`dtnprint`, `dtntrigger`, and `dtnping` were migrated from `register_application` (Application trait) to `register_service` (low-level `BpaService` trait). This provides `on_receive` with the raw CBOR wire bundle bytes (`Bytes`) rather than just the decoded payload, enabling:
- Full BPSec BIB block detection and verification via `CheckedBundle::parse`.
- Access to the security source EID embedded in the BIB.
- Payload extraction directly from the parsed Payload block (block number 1).

### 2. Support HMAC-SHA256 only (RFC 9173 BIB-HMAC-SHA2)

Only BIB HMAC-SHA256 (`HMAC_SHA2` context, `HS256` key algorithm) is supported for signing and verification, as this is the only BPSec integrity context standardized in RFC 9173 and implemented in `hardy-bpv7`. Ed25519 and other asymmetric algorithms are intentionally excluded pending future RFC standardization.

### 3. TOML-based keystore with EID pattern matching

Key material is stored in a TOML file at `~/.config/dtn/keystore.toml` (or a custom path via `--keystore`). Each entry maps an EID pattern (supporting `*` suffix wildcards) to key bytes. This allows flexible multi-node key management without per-bundle key lookup configuration.

```toml
[[keys]]
eid = "dtn://node1/*"
key = "my-secret-key"

[[keys]]
eid = "dtn://trusted-relay/*"
key_file = "/etc/dtn/relay.key"
```

### 4. `CapturingKeySource` for BPSec security source EID resolution

The `CheckedBundle::parse` API invokes a `KeySource` factory closure with the security source EID used by each BIB block (which may differ from the bundle source EID). A custom `CapturingKeySource` wrapper captures this EID through an `Arc<Mutex<Option<Eid>>>` shared with the outer verification logic, allowing accurate key selection for multi-hop signed bundles.

### 5. Three-mode verification policy

All receiving utilities expose a `--verify-policy` argument with three modes:
- `strict`: Bundles that are unsigned, have an invalid signature, or have an unknown security source key are silently dropped.
- `warn` (default): Warnings are emitted to `stderr` but the bundle payload is still processed.
- `ignore`: No BPSec verification is performed.

### 6. Outgoing signing for `dtnsend` and `dtnping`

`dtnsend` and `dtnping` support optional BPSec BIB signing of outgoing bundles via `--sign-key` / `--sign-key-file` arguments. The bundle is built locally using `hardy_bpv7::builder::Builder`, then signed using `dtn_hdy_utils::security::sign_bundle` before being dispatched. An optional `--security-source` EID can be specified to override the default (bundle source EID).

### 7. `SignAlg` enum retained as forward-compatibility placeholder

Although only `HmacSha256` is currently supported, the `SignAlg` enum in `security.rs` is retained in the keystore configuration to allow future extension when new BPSec contexts are standardized, without requiring a breaking change to keystore file formats.

## Consequences
- **Security**: All DTN bundles passing through `dtnprint`, `dtntrigger`, and `dtnping` can be cryptographically authenticated end-to-end using RFC 9173 HMAC-SHA256.
- **Compatibility**: The `warn` default policy ensures backwards compatibility with legacy unsigned deployments.
- **Correctness**: The `CapturingKeySource` pattern correctly resolves the security source EID even when it differs from the bundle source EID.
- **Maintainability**: Security logic is centralized in `src/security.rs` and shared across all utilities via the `dtn_hdy_utils` library crate.
- **Scope limitation**: Full bundle encryption is intentionally not supported; only bundle authentication (integrity) is provided. This is consistent with the ham radio regulatory constraint that forbids encryption.
