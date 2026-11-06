# ADR 15: KISS Transport Leaky-Bucket Rate Limiter & Half-Duplex Checkpoint Pacing

## Status
Accepted

## Context
Amateur radio and satellite DTN deployments using the Licklider Transmission Protocol (LTP, RFC 5326) over KISS framing (TCP or serial TNCs) present specific physical and link-layer constraints:
1. **Low-Memory Hardware TNCs:** Microcontroller-based TNCs (e.g. TNC-X, Mobilinkd, Kantronics) often have small FIFO receive buffers (1–8 KB). A rapid burst of segments from the host can overflow the TNC buffer before it is modulated onto RF.
2. **Half-Duplex (Simplex) Radio Channels:** In simplex radio operations (a single shared frequency for TX and RX), a transceiver is "deaf" while transmitting (PTT active). When an LTP transmitter reaches a Checkpoint (CP), the remote receiver must transmit a Report Segment (RS). If the local transmitter continues to transmit data (e.g., green data, subsequent bundles, or other sessions), it monopolizes the channel, preventing the peer from delivering the RS and causing retransmission timeouts ($RTO_{CP}$).
3. **Loss of Signal & Satellite Eclipse:** On fading channels or low-Earth orbit (LEO) satellite passes, a connection may abruptly drop. Any stop-and-wait mechanism must not hang indefinitely if the peer's RS never arrives.

## Decision
We introduce two orthogonal, independently configurable flow-control mechanisms in the `hardy-ltp-cla` transport layer:

### 1. Global Leaky-Bucket Rate Limiter
- **Scope:** Global per KISS transport interface (shaping the aggregate byte stream emitted to the TNC).
- **Configuration Parameters:**
  - `rate_limit_bytes_per_sec`: `Option<u64>` (e.g., 1200 or 9600 bps equivalent). If unset, transmission is unthrottled.
  - `burst_capacity_bytes`: `Option<usize>` (maximum burst size in bytes). Defaults to a sensible TNC buffer ceiling (e.g., 2048 bytes).
- **Behavior:** Ensures output transmission matches the physical channel baudrate and prevents hardware TNC buffer overrun without requiring priority queues at the low-level byte stream sink.

### 2. Half-Duplex Checkpoint Pacing (Turnaround / Quiet Window)
- **Scope:** Interface-wide transmission queue freeze upon transmitting any LTP Checkpoint segment (Data segment types 1, 2, or 3).
- **Modes:**
  - **Dynamic Event-Driven (`WaitForReport`, Default):**
    - The outbound transmission queue on the KISS port pauses immediately after emitting a Checkpoint.
    - Transmission resumes immediately upon receipt of an incoming Report Segment (RS, type 8) for that session.
    - A safety fallback timeout (`turnaround_timeout: Duration`, default 3s) unblocks the queue if no RS is received (e.g. satellite eclipse, propagation drop, lost packet).
  - **Fixed Silence (`FixedDwell`):**
    - The outbound transmission queue pauses for a fixed duration (`dwell_time: Duration`, e.g. 500ms) after every Checkpoint to release PTT and allow peer turnaround, then automatically resumes.
- **Independence:** The Leaky-Bucket and the Checkpoint Pacing mechanisms are completely decoupled and can be used individually, together, or neither (full-duplex unthrottled).

### 3. TNC Control Delegation
- Pure software application-level pacing in `hardy-ltp-cla`.
- No raw KISS control commands (e.g. `0x01 TXDELAY`, `0x05 FullDuplex`) are generated on the link; hardware TNC settings remain managed by the operator or host TNC configuration.

## Consequences
- Protects low-cost amateur radio TNC hardware from buffer exhaustion.
- Resolves simplex frequency collisions between outbound data and inbound Report Segments in LTP.
- Configurable per transport instance in YAML configuration files.
