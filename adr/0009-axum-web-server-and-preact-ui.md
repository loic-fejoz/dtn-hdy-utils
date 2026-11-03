# 0009. Axum Web Server and Preact UI for `dtn-hdy-ham`

Date: 2026-11-03

## Status

Accepted

## Context

The `dtn-hdy-ham` standalone daemon provides an integrated DTN BPv7 bundle processing agent designed for amateur radio operators. While it already exposes a gRPC interface for low-level client utilities (`dtnsend`, `dtnprint`), operators require a user-friendly, lightweight web dashboard and API.
In the long term, this interface will align with the feature set of the companion mobile application `dtn-android-messenger` (chat, bundle inspection, SenML telemetry, file exchange, opportunistic sender).

For this initial workpackage, the daemon must expose:
1. An embedded HTTP API powered by [Axum](https://github.com/tokio-rs/axum).
2. A single-page application (SPA) built using Vite, Preact, and Tailwind CSS.
3. Read-only presentation of the active station configuration.
4. Generic bundle submission supporting text and binary payloads.

## Decision

1. **Web Server Architecture**:
   - Integrate an optional `web` configuration block in `HamConfig`:
     ```yaml
     web:
       address: "127.0.0.1:8080"
       static-dir: "dtn-hdy-ham/web/dist"
     ```
   - If present, `HamServer` spawns an asynchronous `axum` HTTP server on the configured address.
   - The web server serves the static assets from `static-dir` via `tower_http::services::ServeDir`, with a fallback to `index.html` to support client-side routing.

2. **API Design (`/api/v1/`)**:
   - `GET /api/v1/config`:
     Returns the active, normalized station configuration in structured JSON format (station callsign, primary node ID, admin endpoints, gRPC address, storage mode, configured CLAs with their peers and contact plans).
   - `POST /api/v1/bundles`:
     Accepts both `application/json` (for text messages) and `multipart/form-data` (for binary file uploads or raw payloads).
     Fields:
     - `destination`: Required destination EID (e.g. `dtn://statb/incoming`).
     - `source`: Optional source EID (defaults to local node EID `dtn://<callsign>/`).
     - `lifetime`: Optional bundle lifetime in seconds (defaults to 3600).
     - `payload`: Text content (JSON) or file stream (multipart).
     Returns `{ "bundle_id": "<key>", "status": "sent" }`.

3. **Bundle Injection Engine**:
   - Bundles submitted through the web API are constructed via `hardy_bpv7::builder::Builder` and injected into the in-process BPA instance via an internal `Cla` sink adapter, ensuring immediate routing through the active CLAs.

4. **Frontend Stack**:
   - Colocated at `dtn-hdy-ham/web/`.
   - Tooling: `pnpm` + Vite + Preact + Tailwind CSS.
   - Responsive design with dark/light mode toggle inspired by the Android Messenger interface (`../dtn-android-messenger`).
   - Two core views for this workpackage:
     - **Configuration**: Status cards summarizing active callsign, node ID, storage, gRPC, and CLAs with peer schedules.
     - **Send Bundle**: Interactive form to compose text or upload files with real-time feedback and delivered bundle ID.

## Consequences

- Operators can inspect and send DTN bundles from any web browser without needing terminal utilities or separate client binaries.
- The web application is decoupled from the Rust backend via a standard versioned REST API (`/api/v1/`), allowing independent testing and rapid evolution toward chat and bundle listing.
- Requires Node.js/pnpm for building frontend assets during release packaging, while maintaining zero overhead when `web` is omitted in configuration.
