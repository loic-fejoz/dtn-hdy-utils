# 0012. Dedicated Interactive Chat Service for `dtn-hdy-ham`

Date: 2026-11-04

## Status

Accepted

## Context

Amateur radio operators using Delay/Disruption-Tolerant Networking (DTN BPv7 - RFC 9171) require a direct, conversational communication medium tailored to tactical field messaging and asynchronous station-to-station exchanges.
The native mobile companion app `dtn-android-messenger` features a conversational **Chat service** operating over the standard endpoint `dtn://<callsign>/chat`.

To achieve feature parity in the `dtn-hdy-ham` web console while strictly adhering to amateur radio constraints and RFC 9171 specifications:
1. The daemon must automatically register a permanent local service `dtn://<station>/chat` at startup so that incoming chat bundles destined for the station are received, stored, and processed.
2. Messages must be organized into conversation threads grouped by peer node EID (e.g., `dtn://f4jxq/`).
3. Conversation histories, message directions (`inbound` / `outbound`), timestamps, and read states must be durably persisted across daemon restarts.
4. Payload format must be plain UTF-8 text rendered as Markdown with emoji support for full interoperability with `dtn-android-messenger`.
5. Outgoing transmission status tracking must be realistic and reflect DTN store-and-forward behavior:
   - `queued` (⏳): Stored locally in BPA spool awaiting link availability / CLA forwarding.
   - `sent` (📤): Successfully forwarded over CLA (TCPCLv4, LTP, HQFBP, Mercury) to the next hop.
   - Avoid artificial "delivered" status on DTN links where automatic return receipts are omitted to prevent unnecessary RF bandwidth consumption.
6. The web console must feature a dedicated dual-pane Chat interface placed in the second navigation position:
   `[ ⚙️ Configuration ] [ 💬 Chat ] [ 📦 Bundles ] [ ✉️ Send Bundle ]`.
7. Real-time updates must be pushed instantly to the client via Server-Sent Events (`/api/v1/events`) using a dedicated `chat_message` event type.

## Decision

1. **Local Chat Service Registration (`ChatService`)**:
   - `dtn-hdy-ham` registers a built-in permanent service on `hardy_bpv7::eid::Service::Dtn("chat".into())` during `HamServer::run`.
   - Incoming bundles delivered to `dtn://<node_id>/chat` are extracted, parsed as UTF-8 Markdown text, stored in the chat database, and broadcast via SSE.

2. **Dedicated Internal Services Storage Engine (`internal_storage.rs` / `services.db`)**:
   - Backed by an embedded SQLite database (`services.db`) in the station's dedicated internal storage subdirectory (or an in-memory database in ephemeral mode).
   - Schema:
     ```sql
     CREATE TABLE IF NOT EXISTS chat_conversations (
         peer TEXT PRIMARY KEY,
         created_at TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS chat_messages (
         id TEXT PRIMARY KEY,
         conversation_peer TEXT NOT NULL,
         sender_eid TEXT NOT NULL,
         recipient_eid TEXT NOT NULL,
         content TEXT NOT NULL,
         direction TEXT NOT NULL CHECK(direction IN ('inbound', 'outbound')),
         status TEXT NOT NULL CHECK(status IN ('queued', 'sent', 'received')),
         is_read INTEGER NOT NULL DEFAULT 0,
         timestamp INTEGER NOT NULL,
         created_at TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_chat_peer ON chat_messages(conversation_peer, timestamp);
     ```
   - Persists conversations explicitly in `chat_conversations` so newly opened chats survive daemon restarts even before any message is sent.

3. **Target Normalization & Distinct Conversation Identity**:
   - Target normalization rules:
     - If the target starts with `dtn://`, check whether a service is present in the URI path. If no service is present (e.g. `dtn://f4jxq-9` or `dtn://f4jxq-9/`), append `/chat`. If a service is already present (e.g. `dtn://f4jxq-9/chat` or `dtn://f4jxq/stats`), do not modify anything.
     - If the target does not start with `dtn://`, prepend `dtn://` and append `/chat` (e.g. `f4jxq-9` -> `dtn://f4jxq-9/chat`).
   - Distinct conversation identity:
     - No "smart" stripping of SSID or service: the conversation peer key is the full EID including service.
     - `dtn://f4jxq-9/chat`, `dtn://f4jxq/stats`, and `dtn://f4jxq/chat` represent 3 distinct conversations.

4. **REST API Endpoints (`/api/v1/chat`)**:
   - `GET /api/v1/chat/conversations`: Returns a summary list of all conversation partners, last message snippet, timestamp, and unread count.
   - `POST /api/v1/chat/conversations`: Explicitly creates/registers a persistent conversation thread in `chat_conversations`.
   - `GET /api/v1/chat/messages?peer={peer}`: Retrieves chronological message history for a given correspondent.
   - `POST /api/v1/chat/messages`: Sends a message addressed to the target endpoint, registers outgoing message in `chat_messages` as `queued`/`sent`, dispatches bundle via `WebBundleInjector`, and notifies clients.
   - `POST /api/v1/chat/read`: Marks all inbound messages from a correspondent as read.

5. **Real-time SSE Notification**:
   - On message reception or dispatch, emits `chat_message` on the SSE event channel (`/api/v1/events`).
   - Browser client dynamically appends the new message bubble to active threads and increments unread indicators on inactive threads.

6. **Frontend User Interface (`ChatView.tsx`)**:
   - Responsive dual-pane layout:
     - Left pane: Conversation list, last message preview, unread badge, and "+ New Chat" button.
     - Right pane: Active thread header with exact correspondent EID (`selectedPeer`), scrollable message bubble thread (received on left, sent on right with ⏳ / 📤 status), and an instant message input box.
   - "+ New Chat" modal: Autocompletes known contacts/peers, shows real-time target normalization preview, and persists the conversation to `services.db` via `POST /api/v1/chat/conversations`.

## Consequences

- Full conversational feature parity with `dtn-android-messenger` for operators using the `dtn-hdy-ham` web UI.
- No redundant RF traffic generated by avoiding fake delivery status loops.
- All conversations persist reliably across restarts without polluting the low-level raw bundle inspection table.
- Reactive real-time experience without periodic polling.
