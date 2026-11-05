# ADR 0013: Dedicated Internal Files Service, BUNDLE_LIST Viewer & Sticky Retention Policy

## Status
Accepted

## Context

Following the implementation of the permanent Chat service (ADR 0012), the station requires an internal storage and inspection service for file bundles exchanged over amateur radio links.
In the companion Android application `dtn-android-messenger`, this pattern is represented by the `BUNDLE_LIST` viewer, which provides:
1. **Level 1 (Bundle List)**: Chronological inbox of received bundles for a service endpoint, displaying sender, date, payload size, read status, and retention flags.
2. **Level 2 (Bundle Detail View)**: Detailed metadata inspection (bundle ID, source, destination, creation timestamp, lifetime, hop count) paired with multi-format content rendering:
   - Markdown documents with full UTF-8, emojis, and embedded base64 Data-URI images (`data:image/...`).
   - Image files (`image/png`, `image/jpeg`, `image/gif`, `image/webp`).
   - Audio recordings (`audio/*`).
   - Hexadecimal dumps for arbitrary binary payloads.
   - Quick action controls: 📌 Toggle Sticky, 💾 Download/Save, ✉️ Reply, and 🗑️ Delete.

Additionally, store-and-forward nodes in amateur radio environments cannot accumulate delivered bundles indefinitely on disk or in RAM. When a bundle's lifetime expires (`creation_time + lifetime < now`), it must be deleted unless an operator explicitly protects it by marking it as **`sticky`**. This policy applies uniformly to all internal station services (`files` and `chat`).

## Decisions

1. **Permanent Files Service (`FilesService`)**:
   - `dtn-hdy-ham` registers a built-in permanent service on `hardy_bpv7::eid::Service::Dtn("files".into())` during server startup.
   - Incoming bundles delivered to `dtn://<station>/files` are parsed, raw payloads are stored on disk under `<data_dir>/internal_services/files/` (or in memory in ephemeral mode), and metadata records are stored in a dedicated SQLite database.

2. **Dedicated Internal Services Storage (`services.db`)**:
   - Placed in a dedicated sub-directory `<data_dir>/internal_services/services.db`.
   - Table `files_bundles`:
     ```sql
     CREATE TABLE IF NOT EXISTS files_bundles (
         id TEXT PRIMARY KEY,
         source_eid TEXT NOT NULL,
         destination_eid TEXT NOT NULL,
         creation_timestamp INTEGER NOT NULL,
         lifetime_secs INTEGER NOT NULL,
         expiry_timestamp INTEGER NOT NULL,
         size INTEGER NOT NULL,
         payload_path TEXT NOT NULL,
         mime_type TEXT NOT NULL,
         is_sticky INTEGER NOT NULL DEFAULT 0,
         is_read INTEGER NOT NULL DEFAULT 0,
         created_at TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_files_expiry ON files_bundles(is_sticky, expiry_timestamp);
     ```
   - Table `chat_messages`: enhanced with `is_sticky INTEGER NOT NULL DEFAULT 0` and `expiry_timestamp INTEGER NOT NULL DEFAULT 0`.

3. **CPU-Efficient Cyclic Expiration Engine**:
   - An asynchronous background task queries the earliest expiration time across non-sticky items in `files_bundles` and `chat_messages`:
     `SELECT MIN(expiry_timestamp) FROM ... WHERE is_sticky = 0 AND expiry_timestamp > now`.
   - The task sleeps until that exact wall-clock deadline (`tokio::time::sleep`), eliminating busy polling cycles.
   - Upon wake-up, expired non-sticky bundle records and payload files are deleted. Degraded conditions (missing file, I/O permission denied, database errors) are logged without crashing the daemon.

4. **REST API Endpoints (`/api/v1/files`)**:
   - `GET /api/v1/files`: Lists received bundles with metadata.
   - `GET /api/v1/files/{id}`: Returns complete bundle metadata and text preview.
   - `GET /api/v1/files/{id}/payload`: Downloads raw binary payload with inferred `Content-Type` and `Content-Disposition`.
   - `POST /api/v1/files/{id}/sticky`: Toggles the sticky retention flag.
   - `POST /api/v1/files/{id}/read`: Marks the bundle as read.
   - `DELETE /api/v1/files/{id}`: Deletes the bundle and removes its payload file.
   - `POST /api/v1/chat/messages/{id}/sticky`: Toggles sticky status for chat messages.

5. **Frontend Navigation & UI (`FilesView.tsx`)**:
   - Tab positioned 3rd: `[ ⚙ Configuration ] [ 💬 Chat ] [ 📁 Files (N) ] [ 📦 Bundles ] [ ✉ Send Bundle ]`.
   - Unread counter badge displayed on `[ 📁 Files ]`.
   - Native browser History integration via Hash fragment (`#/files` for list, `#/files/:id` for detail), enabling back/forward browser navigation without page reloads.
   - Markdown renderer supporting UTF-8, emojis, and inline Data-URI images (`data:image/png;base64,...`).

## Consequences

- Full functional alignment with the `BUNDLE_LIST` viewer of `dtn-android-messenger`.
- Storage is protected against disk exhaustion through automatic lifetime-based expiration, while critical bundles can be preserved indefinitely with the `sticky` flag.
- Smooth browser navigation experience through the History API and hash fragments.
- Zero CPU waste due to deadline-based sleeping rather than high-frequency polling.
