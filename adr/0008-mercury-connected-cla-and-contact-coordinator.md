# ADR 8: Mercury Connected CLA and Shared Contact Coordinator

## Status
Accepted

## Context
Deploying DTN BPv7 in amateur radio (HAM) environments requires leveraging real-time modems (such as Mercury TNC) over HF/VHF channels. Mercury offers both broadcast modes (using KISS over TCP) and reliable connected ARQ modes (over dedicated TCP control and data ports, typically 8300 and 8301).

Amateur radio operations over scheduled HF links typically rely on contact schedules (recurring cron schedules or planned time windows) to bring up links, exchange traffic, and disconnect cleanly.
Furthermore, the RADIANT project proposal specifies that node identifiers for amateur radio operators should naturally default to `dtn://<callsign>/`, avoiding redundant or repetitive configuration across the node and convergence layers.

## Decisions

### 1. Dedicated CLA Crate: `hardy-mercury-cla`
A dedicated crate `mercury-cla/` (package `hardy-mercury-cla`) implements the execution endpoint for Mercury's connected ARQ mode:
- **Control Interface (TCP 8300)**: Issues `CONNECT <my_callsign> <peer_callsign>` and `DISCONNECT` commands; monitors for status responses (`CONNECTED <my_call> <peer_call> <bps>`, `DISCONNECTED`).
- **Data Interface (TCP 8301)**: Carries bundle traffic.
- **Wire Framing**: 4-byte big-endian `u32` length-prefixed raw CBOR bundles (`[Length: 4 bytes][Bundle CBOR]`).
- **Bidirectional Transfer**: Operates full-duplex session handling—allowing concurrent transmission and reception over the ARQ data stream while connected.

### 2. Reusable `ContactCoordinator`
A shared `ContactCoordinator` module is introduced in `dtn-hdy-ham/src/coordinator/`:
- **Execution Endpoint Separation**: The CLA acts as a pure execution endpoint (`connect_peer`, `disconnect_peer`, `send_bundle`). The coordinator orchestrates the contact lifecycle.
- **Timing Engine**: Supports recurring contacts (5- or 6-field `cron` expressions with a `duration`) and `one-shot` scheduled windows (`start` and `end` timestamps).
- **Graceful Teardown with Hard Backstop**:
  - `grace-period` (e.g. 30s): Allows the in-flight bundle transmission to complete over the ARQ channel after the contact duration expires before issuing `DISCONNECT`.
  - `idle-timeout` (e.g. 20s): Closes the session early if both TX and RX queues remain idle, freeing the radio frequency.
- **Routing Integration**: Integrates directly with Hardy's standard peer registration (`sink.add_peer(&node_id)` upon connection, `sink.remove_peer(&node_id)` upon disconnect), ensuring all existing routing mechanisms in Hardy remain unaffected.
- **Multi-CLA Reuse**: Designed to be shared with `hardy-hqfbp-cla` for scheduled file broadcast contacts.

### 3. Simplified Configuration & RADIANT Callsign Defaults
Following the RADIANT project conventions:
- **Root-level `callsign`**: Specifying `callsign` at the top level of `HamConfig` automatically defaults the administrative node EID to `dtn://<callsign>/` (lowercased) if `node-id` is omitted.
- **CLA Callsign Inheritance**: The top-level `callsign` is automatically inherited as the default `my-callsign` for all CLAs.
- **Peer `callsign` & `node-id` Defaulting**: For CLA peers, specifying `callsign` automatically defaults the peer's `node-id` to `dtn://<callsign>/`.

Example simplified configuration:
```yaml
callsign: "STATA" # Defaults node-id to dtn://stata/

clas:
  - name: "mercury-arq"
    type: "mercury-arq"
    control-addr: "127.0.0.1:8300"
    data-addr: "127.0.0.1:8301"
    idle-timeout: "20s"
    grace-period: "30s"
    peers:
      - callsign: "STATB" # Defaults node-id to dtn://statb/
        schedule:
          type: "recurring"
          cron: "0 */15 * * * *"
          duration: "3m"
```

## Consequences
- Clean separation between transport execution (`hardy-mercury-cla`) and timing/scheduling (`ContactCoordinator`).
- Minimal configuration burden for amateur radio operators through RADIANT callsign defaulting.
- Future-proof design allowing both `hardy-mercury-cla` and `hardy-hqfbp-cla` to leverage the same schedule coordinator.
