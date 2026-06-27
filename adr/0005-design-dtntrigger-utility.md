# ADR 5: Design and Strategy for `dtntrigger` Utility

## Status
Proposed

## Context
We need to implement a bundle trigger utility named `dtntrigger` for the `dtn-hdy-utils` suite. This tool should mirror the behavior of the `dtn7-rs` `dtntrigger` utility:
- Subscribes to a specific DTN service endpoint.
- For every received bundle payload, it can either print it directly (with `--print` flag) or run a user-specified command (passing the bundle source EID and the path to a temporary file containing the payload bytes).
- Maintains compatibility with command-line flags and behavior of `dtn7`'s trigger tool while integrating with the `hardy` BPA's gRPC services.

## Decision
We will implement the `dtntrigger` utility with the following architectural choices:

1. **Hardy BPA Connection and Registration**:
   - Use the `RemoteBpa::register_application` gRPC interface (similar to `dtnprint`) to dynamically register an application handler for the requested service.
   - We will parse the `--endpoint` argument as a `Service::Ipn` if it's a valid integer, and `Service::Dtn` otherwise.

2. **CLI Flags Parity**:
   - Provide the exact flags supported by `dtn7`'s `dtntrigger`:
     - `-p`, `--port`: Local gRPC port of Hardy BPA (defaulting to `50051` for Hardy compatibility, overriding with `DTN_WEB_PORT` env var).
     - `-6`, `--ipv6`: Connect to localhost using IPv6 (`[::1]`).
     - `-v`, `--verbose`: Enable logging of connection state and bundle information.
     - `-e`, `--endpoint`: The local endpoint name or number to listen on.
     - `--print`: Directly print incoming payloads to standard output (formatted as `[<rfc3339_timestamp>] <source_eid> → <payload_text>`).
     - `-c`, `--command`: Shell command to execute (defaults to `"echo"`), splitting it by whitespace, starting the executable, and appending `<source_eid>` and `<payload_file_path>`.

3. **Temporary File Management**:
   - Add the `tempfile` crate as a dependency in `Cargo.toml`.
   - Write incoming bundle payload bytes to a `tempfile::NamedTempFile` immediately before running the command, which guarantees clean, secure creation and automatic cleanup once the process ends and the file handle is dropped.

4. **Command Execution**:
   - Split the command string by whitespace. Use `std::process::Command` to invoke the executable.
   - Wait for command execution to complete.
   - If the command fails (non-zero exit status) or if `--verbose` is enabled, write the command's stdout and stderr to the respective terminal streams.

## Consequences
- Parity: The CLI flags and usage of `dtntrigger` will remain compatible with existing DTN7-based scripts and triggers.
- Robustness: Utilizing `tempfile` prevents leftover temporary file clutter on disk and ensures isolation.
- Offline/Online behavior: Since it registers via gRPC, it relies on a running Hardy BPA daemon.
