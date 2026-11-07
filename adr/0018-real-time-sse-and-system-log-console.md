# ADR 18: Real-Time Event Push (SSE) & In-Browser System Log Console

## Status
Accepted

## Context
Amateur radio operators managing their station through `dtn-hdy-ham` need continuous, real-time observability over station activity:
1. **Interactive Messaging and File Transfer:** Currently, operators need to manually refresh or wait on periodic polling loops to notice when a chat message or file attachment has arrived over the radio.
2. **Link Health and Contact Windows:** Radio links (KISS TNC, Mercury ARQ, LTP over UDP, and TCPCLv4) are inherently dynamic and intermittent. Operators need immediate visual indication when a link is established, disconnected, or awaiting a scheduled contact window.
3. **Daemon Diagnostics and Troubleshooting:** When deploying nodes on headless machines (Raspberry Pi, field laptops, repeater cabinets), diagnosing routing decisions, CLA framing issues, or handshake failures requires terminal access (`journalctl` or `docker logs`). An in-browser diagnostic console allows field operators to inspect daemon diagnostics from any connected device (phone, tablet, PC).

## Decision

We introduce a unified real-time event pipeline using Server-Sent Events (SSE) and an in-browser live System Log Console:

### 1. Mixed Tracing Architecture with In-Memory Ring Buffer
- The standard Rust `tracing_subscriber` logger continues to output to `stderr` with full ANSI color formatting, ensuring seamless compatibility with systemd (`journalctl`), Docker logs, and terminal sessions.
- A custom `tracing_subscriber::Layer` (`LogBufferLayer`) captures tracing events into:
  - An in-memory circular ring buffer holding the **last 500 log entries**.
  - A real-time `tokio::sync::broadcast` channel delivering live entries to connected web clients.
- Log entries are structured with `id`, `timestamp` (RFC 3339 local/UTC), `level` (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`), `target` (module path), and `message`.

### 2. Real-Time SSE Event Stream (`GET /api/v1/events`)
- `GET /api/v1/events` pushes real-time events over HTTP SSE (`text/event-stream`):
  - `services_update`: Active DTN services registered via gRPC or built-in.
  - `bundles_update`: Bundle stored or evicted in BPA / storage.
  - `chat_message`: Incoming or outgoing chat message.
  - `file_received`: Newly received and stored file bundle.
  - `cla_state_changed`: CLA connection state transitions (Connected 🟢, Disconnected 🔴, Dialing 🟡).
  - `contact_window`: Contact coordinator scheduled contact execution.
  - `log_entry`: Live log entries streamed when requested via `GET /api/v1/events?logs=true`.
- **Performance Optimization**: Clients connect without `?logs=true` for regular views to avoid high-frequency log flooding, and request `?logs=true` only when the Log Console is open.

### 3. Log History REST Endpoint (`GET /api/v1/logs`)
- Returns the current snapshot of the 500-entry in-memory ring buffer as a JSON array, allowing instant population of the web console upon opening or reconnecting.

### 4. In-Browser System Log Console (`LogsView.tsx`)
- Accessible via the `# /logs` tab.
- Features:
  - Severity level filter (`ALL`, `INFO`, `WARN`, `ERROR`, `DEBUG`, `TRACE`).
  - Real-time text search filtering across log message and module target.
  - Smart auto-scroll: sticks to the bottom when new logs arrive, automatically pauses if the user scrolls up, with a resume button.
  - Local clear button (clears browser view without affecting server buffer).
  - Export/download button (saves displayed logs to a text file `dtn-logs-<timestamp>.txt`).
  - Live SSE connection indicator (🟢 Live, 🔴 Disconnected, 🟡 Reconnecting).

### 5. Non-Intrusive Audio & Visual Notifications
- Discretely alerts operators when new chat messages or files arrive.
- Audio chime synthesized in-browser via the Web Audio API (`AudioContext`), requiring zero external audio file dependencies.
- Operator toggle in navigation settings (sound on/off), persisted in `localStorage`.

## Consequences
- Operators receive instant feedback on message deliveries and link transitions with zero manual reloads.
- Diagnostic logs are instantly accessible from mobile devices and web browsers without SSH/terminal access.
- Memory overhead is strictly bounded by the 500-entry ring buffer.
