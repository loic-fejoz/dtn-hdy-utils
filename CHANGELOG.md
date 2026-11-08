# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-08

### Added

#### `dtn-hdy-ham` Daemon
- **Standalone Amateur Radio BPA Daemon**: Tailored BPv7 Delay-Tolerant Networking node daemon for amateur radio operators.
- **Embedded Web Management UI & PWA**: Responsive web dashboard built with Preact and Tailwind CSS, featuring PWA offline caching, mobile bottom navigation, and bundle inspector.
- **SenML Telemetry Gateway & Open MCT Dashboard (RFC 8428)**: Ingestion of CBOR/JSON SenML telemetry packs, SQLite timeseries storage, and live NASA Open MCT telemetry visualization.
- **BPSec Security & Keystore (RFC 9172)**: Integrated Block Integrity Block (BIB) signer and verifier using HMAC-SHA256 with EID pattern matching and access control.
- **Chat, Voice Notes & File Transfer**: Delay-tolerant text messaging with adjustable lifetime presets, RFC 7845 Ogg/Opus voice notes, and payload file transfers.
- **Dynamic Routing & Peering**: Gateway peering coordinator supporting unencrypted Hamnet TCPCLv4 (`require-tls: false`) and dynamic static routing.
- **Interactive First-Time Setup Wizard**: `dtn-hdy-ham init` command for guided node configuration.
- **Real-Time SSE Event Stream**: Server-Sent Events endpoint and live log console with visual filtering and levels.

#### Convergence Layer Adapters (CLAs)
- **`hardy-ltp` & `hardy-ltp-cla`**: Pure Rust sans-I/O implementation of the Licklider Transmission Protocol (RFC 5326 / RFC 7122), KISS TNC framing, half-duplex dwell pacing, and leaky-bucket token rate limiter.
- **`hardy-hqfbp-cla`**: Hamradio Quick File Broadcasting Protocol CLA with Reed-Solomon FEC, CRC32/CRC16 verification, chunk fragmentation, and KISS pacing.
- **`hardy-mercury-cla`**: Mercury connected ARQ mode CLA and contact coordinator for reliable packet radio contacts.
- **`cla-commons`**: Shared token-bucket / leaky-bucket pacing and half-duplex link modeling utilities.

#### BPv7 CLI Utilities Suite
- **`dtnsend`**: Simple bundle transmission utility supporting payload pipes and metadata flags.
- **`dtnprint`**: Bundle receiver displaying payload contents and header details.
- **`dtnquery`**: Bundle inspection and database status query client.
- **`dtntrigger`**: Event-driven bundle receiver executing shell commands upon bundle receipt.
- **`dtnping`**: Delay-tolerant ping utility measuring RTT over DTN hops.
- **`dtnfiles`**: Out-of-band file distribution client.
- **`dtnforward`**: Store-and-forward bundle relaying service.
- **`dtnbib`**: BPSec Block Integrity Block cryptographic validator.
- **`dtnbasket` & `dtnbasket-cli`**: Delay-tolerant resource proxy service and client (draft-f4jxq-dtn-basket-00).
- **`hdy-stats`**: Metrics collector and status tailing utility.

#### CI/CD & Multi-Platform Distribution
- **Automated Multi-Platform Packaging**: GitHub Actions release workflow assembling standalone static distributions:
  - Linux `x86_64-unknown-linux-musl` (fully static)
  - Linux `aarch64-unknown-linux-musl` (cross-compiled static)
  - Windows `x86_64-pc-windows-msvc`
  - macOS `universal` (arm64 + x86_64 combined binary)
- **Software Bill of Materials (SBOM)**: Automated SPDX and CycloneDX SBOM generation.

### Fixed
- Fixed missing web dist assets in release archives and automated relative static directory discovery.
- Fixed encoding deserialization validation in `hardy-hqfbp-cla`.
- Removed broken symlink `examples/dtnsend` preventing Windows and macOS release packaging.
- Hardened CI and Release pipelines with Protoc download fallback and SBOM error tolerance.
- Addressed security, accessibility, and OWASP recommendations in the web frontend.

---

## [0.1.0] - 2026-08-16

### Added
- Initial project skeleton and `dtn-fortune` utility.
