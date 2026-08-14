# Agent Router Guide

## Mission
Provide command-line utilities (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, and `hdy-stats`) for interacting with the `hardy` BPA implementation of DTN BPv7.
The source code of the `hardy` project is located at `../hardy`.
The source code of the `DTN7` project is located at `../dtn7-rs`.

## Critical Commands
Always run verification tests and check formatting before final delivery:
- **Build**: `cargo build`
- **Test**: `cargo test`
- **Lint**: `cargo clippy && cargo fmt`

## Directory Map
- `src/bin/`: Main executable utilities (`dtnprint`, `dtnsend`, `dtnquery`, `dtntrigger`, `dtnping`, `dtnfiles`, `dtnforward`, `dtnbib`, and `hdy-stats`).
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
> You must ALWAYS verify your work by running the complete command check and test suites (`cargo test` and lint checks) before finishing any task.
