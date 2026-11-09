# Agent Router Guide

## Mission
Provide command-line utilities (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, `dtnbasket`, `dtnbasket-cli`, `hdy-stats`, and `dtn-hdy-ham`), along with dedicated amateur radio convergence layers (`hardy-ltp`, `hardy-ltp-cla`, `hardy-hqfbp-cla`, `hardy-mercury-cla`), for interacting with and deploying the `hardy` BPA implementation of DTN BPv7.
The source code of the `hardy` project is located at `../hardy`.
The source code of the `DTN7` project is located at `../dtn7-rs`.

## Critical Commands
Always run verification tests and check formatting before final delivery:
- **Build**: `cargo build`
- **Test**: `cargo test`
- **Lint**: `cargo clippy && cargo fmt`
- **Panic & Unwrap Guardrails**: Production code (`--lib`, `--bins`) MUST be 100% `unwrap`-free and `panic!`-free. Use `?`, `.ok_or_else()`, or poison-safe mutex access.
- **Security & Quality**: `./scripts/check-security.sh` (or `./scripts/check-security.sh --strict`)
- **CI Sync**: Whenever updating `hardy` dependencies or signatures, update the `hardy` checkout `ref` in `.github/workflows/ci.yml`.

## Directory Map
- `src/bin/`: Main client utility CLIs (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, `dtnbasket`, `dtnbasket-cli`, and `hdy-stats`).
- `dtn-hdy-ham/`: Dedicated standalone BPA binary and server daemon for amateur radio operators.
- `ltp/`: Core Licklider Transmission Protocol (RFC 5326) engine crate (`hardy-ltp`).
- `ltp-cla/`: LTP Convergence Layer Adapter for Hardy (`hardy-ltp-cla`).
- `hqfbp-cla/`: Hamradio Quick File Broadcasting Protocol CLA for Hardy (`hardy-hqfbp-cla`).
- `mercury-cla/`: Mercury TNC ARQ Connected Mode Convergence Layer for Hardy (`hardy-mercury-cla`).
- `examples/`: Example scripts and usage demonstrations for the utilities.
- `agent_docs/`: Reference documentation for autonomous developers.
- `adr/`: Architectural Decision Records (ADRs) tracking design and implementation choices.

## Documentation Index
Read these files progressively based on your task context:
1. [agent_docs/architecture.md](agent_docs/architecture.md): Read before changing client registration or data flow logic.
2. [agent_docs/conventions.md](agent_docs/conventions.md): Read before writing new code, options, or refactoring CLI arguments.
3. [agent_docs/testing_guidelines.md](agent_docs/testing_guidelines.md): Read before adding or validating tests.
4. [adr/README.md](adr/README.md): Index of all Architectural Decision Records.
5. [agent_docs/deployment.md](agent_docs/deployment.md): Deployment guide on remote architectures and using systemd services.

## Verification
> [flat]
> [!IMPORTANT]
> You must ALWAYS verify your work by running the complete command check and test suites (`cargo test`, lint checks `cargo clippy && cargo fmt --check`, security guardrails `./scripts/check-security.sh`, and ensuring `.github/workflows/ci.yml` dependencies are synchronized) before finishing any task.

