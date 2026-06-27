# ADR 6: Design and Strategy for `dtnping` Utility

## Status
Accepted

## Context
We need to implement a Bundle Protocol 7 diagnostic and testing tool named `dtnping` for the `dtn-hdy-utils` suite. This tool should mirror the behavior of Hardy's built-in ping utility (from the `hardy` project `/home/loic/projets/hardy`), measuring round-trip time (RTT) and tracking network path hops via status reports.
However, unlike Hardy's command-line tool which acts as a standalone node (establishing a Convergence Layer Adapter (CLA) directly to the target/peer), this utility must connect as a client to an active local running Hardy BPA instance via gRPC, registering as an application.

## Decision
We will implement the `dtnping` utility with the following architectural choices:

1. **Local Application Registration**:
   - Instead of embedding a local BPA daemon and registering CLAs, the tool connects to the local Hardy instance over gRPC using `RemoteBpa`.
   - It registers as a high-level application implementing the `Application` trait.
   - Because the Hardy gRPC proxy requires a `service_id` for registration (failing if none is provided), the tool generates an ephemeral service ID (e.g. `dtnping-<pid>`) when the user does not specify a custom source endpoint with `-S / --source`.

2. **CBOR Ping Payload Structure**:
   - Embeds a sequence number and optional padding bytes in the payload to match the CBOR array structure `[sequence, options_map]` expected by Hardy's built-in echo service.
   - Implements a lightweight, zero-dependency CBOR encoder/decoder inside the binary to keep the codebase clean and avoid transitive version mismatches.

3. **Status Report Tracking**:
   - The tool sets all notification flags in `SendOptions` (`notify_reception`, `notify_forwarding`, `notify_delivery`, `notify_deletion`, `report_status_time`) to request status reports.
   - When status reports arrive at the BPA, they are parsed and forwarded to the application client via the `on_status_notify` callback.
   - The tool maps `bundle_id` keys returned by `sink.send()` back to sequence numbers, allowing it to print hop transitions (e.g. `Ping 0 forwarded by dtn://node2 after 1.2ms`) and build a visual network path diagram.

4. **RTT & Statistics**:
   - RTT is calculated using local monotonic timers (`std::time::Instant`) rather than timestamps inside the payload, avoiding any clock-synchronization discrepancies between nodes.
   - Tracks statistics (sent, received, min/avg/max/stddev RTT, and loss percentage) and prints them in standard ping summary format upon completion (via count limit, session timeout, or Ctrl+C).

5. **CLI Flags and Parity**:
   - Implements standard options compatible with Hardy's ping tool:
     - `destination`: Position EID to ping.
     - `-c`, `--count`: Number of pings to send.
     - `-i`, `--interval`: Interval between pings (defaults to `"1s"`).
     - `-s`, `--size`: Target bundle size in bytes (MTU testing).
     - `-w`, `--timeout`: Session timeout.
     - `-W`, `--wait`: Grace period to wait for responses after sending all pings.
     - `-q`, `--quiet`: Print summary statistics only.
     - `-t`, `--ttl`: Hop count limit.
     - `--lifetime`: Bundle lifetime.
     - `-S`, `--source`: Register a specific local endpoint instead of a dynamic one.
     - `-p`, `--port` and `-6`, `--ipv6` to connect to the local Hardy daemon.

## Consequences
- **Dynamic Registration**: Registers seamlessly on running multi-node nodes without needing to set up specific convergence layers.
- **Portability**: The tool is self-contained and communicates over standard local gRPC loops.
- **No CLA Overhead**: Operates entirely as a lightweight application layer client.
