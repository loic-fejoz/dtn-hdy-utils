# ADR 17: Unified Static Release Packaging and Out-of-the-Box Distribution

## Status
Accepted

## Context
Deploying DTN applications across diverse radio stations, remote repeaters, and field laptops requires reproducible, zero-dependency static binaries and coherent software distributions.
Previously:
1. `dtn-hdy-ham` had an experimental packaging workflow (`.github/workflows/package-ham.yml`) producing an isolated archive without essential CLI utilities.
2. The core command-line utilities in `src/bin/` (`dtnsend`, `dtnprint`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, `dtnbasket`, `dtnbasket-cli`, `hdy-stats`) were only built during CI test runs and not distributed as precompiled static release assets.
3. Users downloading a release package lacked immediate onboarding assets (ready-to-use configuration templates, quickstart guide, systemd service units), creating friction for new operators.
4. Linux ARM64 (AArch64), a dominant architecture in amateur radio (Raspberry Pi 3/4/5 running 64-bit OS), was not packaged.

## Decision

We establish a unified, automated multi-platform static packaging and release pipeline generating two distinct, coherent distribution packages:

### 1. Two Target Distributions
1. **`dtn-hdy-ham` Package**:
   - Focus: Standalone amateur radio BPv7 node and interactive server.
   - Binaries included in `bin/`:
     - `dtn-hdy-ham`
     - `dtnsend`
     - `dtnprint`
     - `dtnfiles`
     - `dtntrigger`
   - Onboarding assets included:
     - `config.example.toml`: Documented reference configuration file.
     - `QUICKSTART.md`: Step-by-step first-launch guide (daemon, Web UI, CLI tests).
     - `dtn-hdy-ham.service`: Systemd service unit template for Linux daemons.
     - `README.md` and `LICENSE`.

2. **`dtn-hdy-utils` Package**:
   - Focus: Complete DTN client toolkit and protocol testing utilities.
   - Binaries included in `bin/`:
     - `dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, `dtnbasket`, `dtnbasket-cli`, `hdy-stats`.
   - Included assets:
     - `examples/`: Full collection of utility scripts and protocol examples.
     - `README.md` and `LICENSE`.

### 2. Archive Structure
Every archive extracts into a self-contained root directory to avoid polluting the working directory:
```
<package>-<version>-<platform>/
├── bin/
│   ├── <binary1>
│   └── <binary2>...
├── QUICKSTART.md (for ham)
├── config.example.toml (for ham)
├── dtn-hdy-ham.service (for ham)
├── examples/ (for utils)
├── README.md
└── LICENSE
```

### 3. Target Architectures & Static Linking
Four release platforms are built:
1. `linux-x86_64`: Static musl libc (`x86_64-unknown-linux-musl`, `target-feature=+crt-static`).
2. `linux-aarch64`: Static musl libc for 64-bit ARM (`aarch64-unknown-linux-musl`, `target-feature=+crt-static`).
3. `windows-x86_64`: Static MSVC runtime (`x86_64-pc-windows-msvc`, `target-feature=+crt-static`).
4. `macos-universal`: Universal binary combining `x86_64-apple-darwin` and `aarch64-apple-darwin` via `lipo`.

### 4. Release Automation & Checksums
- Orchestrated in a unified workflow `.github/workflows/release.yml`.
- Pushing a Git tag matching `v*` (e.g. `v0.1.0`) automatically publishes a GitHub Release containing:
  - All `.tar.gz` and `.zip` archives for both packages and all 4 platforms.
  - Corresponding `.sha256` checksum files for integrity verification.
- Normal branch pushes generate downloadable workflow artifacts for validation without publishing a GitHub Release.

## Consequences
- New users receive an immediate, frictionless out-of-the-box experience with documented configs and essential utilities bundled with the server.
- Embedded and field operators on Raspberry Pi (ARM64) obtain native, zero-dependency static binaries.
- Advanced DTN developers obtain a dedicated, complete utility package with all 11 CLIs and examples.
