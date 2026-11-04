# 0011. Bundle Storage Inspector & Multi-Format Viewer for `dtn-hdy-ham`

Date: 2026-11-04

## Status

Accepted

## Context

Operators administering an amateur radio DTN BPv7 station via `dtn-hdy-ham` require a unified view to inspect, monitor, download, and delete stored bundles.
The companion native application `dtn-android-messenger` provides a rich bundle viewer capable of rendering various payload formats (plain text, formatted Markdown, images, audio/vocal notes, SenML sensor telemetry, and raw hexadecimal dumps).
To achieve feature parity and provide a dedicated station management console:
1. The web interface must present a single unified table of all stored bundles (with search and status filters).
2. The operator must be able to inspect detailed BPv7 metadata (Primary block, source, destination, creation timestamp, lifetime, status).
3. The operator must be able to preview payloads directly in the browser across all 5 supported types without artificial size truncation for now, or download the raw payload.
4. The operator must be able to delete a bundle from storage directly from the web interface with a confirmation safeguard.
5. Navigation tabs in the navbar must be ordered as: `[ ⚙️ Configuration ] [ 📦 Bundles ] [ ✉️ Send Bundle ]`.
6. Changes to bundles (injections, deliveries, deletions) must push real-time updates over the Server-Sent Events (SSE) `/api/v1/events` stream.

## Decision

1. **Storage Read Engine (`bundle_inspector.rs`)**:
   - Accesses stored metadata via the station's SQLite database (`bundles.db`) using non-blocking read queries (`rusqlite` WAL mode).
   - Reads serialized `StoredBundle` records, resolving canonical EIDs, timestamps, lifetime, status codes (`New`, `Waiting`, `ForwardPending`, `Dispatching`, `WaitingForService`), and `storage_name`.
   - Locates payload files in the configured payload directory (`LocalDiskStorage`), slicing the payload block (block #1) using `Block::payload_range` / `Block::payload`.

2. **Payload MIME & Format Inferrer**:
   - Analyzes header magic bytes to detect:
     - Images: PNG (`89 50 4E 47 0D 0A 1A 0A`), JPEG (`FF D8 FF`), GIF (`GIF87a`/`GIF89a`), WebP (`RIFF....WEBP`).
     - Audio: WAV (`RIFF....WAVE`), OGG (`OggS`), MP3 (`ID3` or sync frame).
     - SenML Telemetry: CBOR / JSON SenML records (RFC 8428) containing `"bn"`, `"n"`, `"v"`, `"vs"`.
     - Text / Markdown: Valid UTF-8 with Markdown indicators.
     - Binary fallback: `application/octet-stream` mapped to the hexadecimal dump viewer.

3. **Backend API Endpoints**:
   - `GET /api/v1/bundles`: List of stored bundle summaries (key, source, destination, size, status, creation timestamp, lifetime).
   - `GET /api/v1/bundles/{id}`: Detailed bundle metadata and detected payload MIME type.
   - `GET /api/v1/bundles/{id}/payload`: Streaming payload endpoint with inferred `Content-Type` and `Content-Disposition`.
   - `DELETE /api/v1/bundles/{id}`: Deletes bundle from SQLite and removes payload file, emitting a `bundles_update` event to all connected SSE clients.

4. **Frontend Architecture (`dtn-hdy-ham/web`)**:
   - Navbar updated with ordered tabs: `Configuration`, `Bundles`, `Send Bundle`.
   - `BundlesView.tsx` component rendering a responsive search-enabled table.
   - Drawer/modal displaying:
     - BPv7 metadata badges (Source, Destination, Size, TTL, Status).
     - Markdown viewer for rich text notes.
     - HTML5 image viewer (`<img>`) and audio player (`<audio controls>`).
     - Formatted SenML / JSON viewer.
     - Byte-by-byte Hex dump viewer (offset, hex bytes, printable ASCII).
     - Action buttons: "Download Raw Payload" and "Delete Bundle" (with confirmation dialog).
   - Connected to SSE `/api/v1/events` listening for `bundles_update` to trigger seamless live re-fetches.
   - For in-memory volatile instances (`storage: in-memory: true`), `dtn-hdy-ham` backs storage with an ephemeral SQLite directory so that bundles in memory remain fully inspectable through the web UI and disappear cleanly on exit.
   - Added `POST /api/v1/clas/connect` endpoint and `ClaManager` paired with an interactive `✋ Connect` button in the Configuration view to force immediate TCPCL peer connection during testing.

## Consequences

- Full observability into local bundle store for amateur radio operators without needing separate CLI tools like `dtnquery`.
- Live updates ensure immediate visibility of incoming bundles over radio CLAs and outgoing bundles dispatched via the web form.
- Both persistent disk storage and in-memory nodes support real-time bundle inspection and multi-format previewing.
- Operators can force CLA peer connections on demand directly from the Configuration tab with immediate status feedback.
- Safe deletion mechanism allows operators to purge problematic or expired bundles manually.
- Clean decoupling between backend storage reads and rich frontend multi-format renderers.
