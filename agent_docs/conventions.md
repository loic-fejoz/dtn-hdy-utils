# Development Conventions

This document outlines key conventions, naming styles, and command usage for developing utilities within this codebase.

## Code Standards & Formatting
Do not define manual formatting constraints or spacing styles in documentation. Always verify styling and run code quality checks using the linter commands:
- Code checks: Run `cargo clippy` and `cargo fmt` to align with Rust workspace requirements.

## CLI Compatibility
- When implementing a command-line utility that already has an equivalent in another DTN implementation (such as `dtn7-rs`), prefer binary CLI compatibility. This means maintaining similar subcommand structure, options, arguments, output formats, and behaviors to make integration and scripting seamless across different DTN implementations.

## Command-Line Arguments & Parsing
- All tools use `clap` with the derive feature for parsing command-line parameters.
- View [dtnprint.rs:L11-L28](../src/bin/dtnprint.rs#L11-L28), [dtnsend.rs:L14-L49](../src/bin/dtnsend.rs#L14-L49), or [dtnping.rs](../src/bin/dtnping.rs) for examples of `Args` clap definitions.
- Port defaults to Hardy gRPC port `50051`.
- Always verify environment variable overrides: check if `HARDY_GRPC_PORT` or `DTN_WEB_PORT` is specified before falling back to CLI defaults. See [dtnsend.rs:L78-L86](../src/bin/dtnsend.rs#L78-L86) for port resolution logic.

## Output Stream Routing
- **`stdout`**: Reserved exclusively for functional outputs like printed bundle payloads, serialized bundle CBOR hex strings, resulting bundle IDs, and ping RTT diagnostics.
- **`stderr`**: Reserved for all logging, status, connection, and debug output (e.g. `Connecting to Hardy BPA...`, `Sending ... bytes`, `Application registered successfully...`). Refer to [dtnprint.rs:L85-L89](../src/bin/dtnprint.rs#L85-L89) for logging streams.

## Time & Timestamp Handling
- Use the `time` crate (specifically `time::OffsetDateTime`) for representing creation/expiry dates.
- Output date string strings formatted with `well_known::Rfc3339` as exemplified in [dtnsend.rs:L210-L216](../src/bin/dtnsend.rs#L210-L216).

## Concurrency & Thread-Safety
- The `on_receive` handler is invoked concurrently when multiple bundles are received.
- Any multi-line output to `stdout` must be written atomically to avoid interleaved text.
- Always acquire an exclusive lock using `std::io::stdout().lock()` before performing stdout writes. See [dtnprint.rs:L55-L62](../src/bin/dtnprint.rs#L55-L62) for the exact reference.
