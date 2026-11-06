# LTP Packet Signatures & Authentication (RFC 5327) TODO

## Context & Objectives
LTP Authentication (RFC 5327) specifies authentication extensions for LTP segments (RFC 5326 §3.1.4) using Message Authentication Codes (MACs) or asymmetric digital signatures.

This checklist tracks the future implementation of packet signing and authentication for `hardy-ltp` and `hardy-ltp-cla`.

---

## Implementation Checklist

### 1. Data Structures & Codec (RFC 5327 §3)
- [ ] **Authentication Extension TLV Parsing & Serialization:**
  - [ ] Define `LtpAuthExtension` representation (Extension Tag `0x00`).
  - [ ] Parse ciphersuite identifier (e.g. HMAC-SHA256-80, HMAC-SHA256, Ed25519).
  - [ ] Parse key identifier (Key ID / SPI) for multi-peer key lookups.
  - [ ] Serialize authentication trailer extension to conform with RFC 5327 §3.
- [ ] **Canonical Digest Input (RFC 5327 §3.2):**
  - [ ] Implement canonical segment byte serialization excluding the authentication extension itself.
  - [ ] Ensure proper byte ordering and SDNV normalization across all segment types.

### 2. Core Engine & State Machine (`hardy-ltp`)
- [ ] **Engine Key Configuration:**
  - [ ] Add peer key ring to `EngineConfig` (`auth_keys: BTreeMap<EngineId, AuthKey>`).
  - [ ] Support authentication policy per peer (`Enforced`, `Optional`, `Disabled`).
- [ ] **Outbound Signing:**
  - [ ] Compute MAC / signature over canonical representation before queuing segment in `outbound`.
  - [ ] Append signed `LtpAuthExtension` to segment trailer extensions.
- [ ] **Inbound Verification & Rejection:**
  - [ ] Verify incoming trailer signature before feeding segment into `handle_incoming_*`.
  - [ ] Silently drop segments with invalid or missing signatures when peer policy requires authentication.
  - [ ] Prevent side-channel timing leaks during MAC verification (constant-time comparison).

### 3. Convergence Layer & Configuration (`hardy-ltp-cla`)
- [ ] **Peer Configuration:**
  - [ ] Expose shared secret / public key configuration in CLA yaml config (`peers[].auth_key`).
  - [ ] Allow amateur radio mode (unauthenticated / clear text) while enabling authentication for commercial/secure backhauls.
- [ ] **Dynamic Key Rotation:**
  - [ ] Support runtime update of keys via management API without interrupting in-flight sessions.

### 4. Tests & Quality Assurance
- [ ] Deterministic unit tests with RFC 5327 reference vectors.
- [ ] Tests verifying rejection of tampered segments (modified payload, altered session number, wrong key).
- [ ] Replay attack mitigation tests (evaluating check against session number / monotonic timestamp).
- [ ] Property-based testing (`proptest`) for auth extension codec and arbitrary corrupt signatures.
