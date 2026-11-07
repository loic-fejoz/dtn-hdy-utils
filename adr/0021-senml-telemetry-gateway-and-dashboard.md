# ADR 21: RFC 8428 SenML Telemetry Ingestion, SQLite Normalization, Open MCT API & Interactive Dashboard

## Status
Accepted

## Context
Amateur radio and DTN networks (BPv7 - RFC 9171) frequently carry IoT and environmental sensor measurements (weather stations, solar battery monitors, APRS/telemetry beacons, balloon payloads). In delay- and disruption-tolerant environments:
1. **RFC 8428 SenML Standard:** Sensor payloads are formatted in Sensor Measurement Lists (SenML) encoded in either JSON (`application/senml+json`) or CBOR (`application/senml+cbor`).
2. **Delay-Tolerant Timestamp Resolution:** Relative timestamps ($t \le 2^{28}$) in SenML must be resolved against the DTN bundle's `creation_timestamp` rather than the local receipt time, ensuring accurate measurement timing regardless of store-and-forward transit delays.
3. **Parity with `dtn-android-messenger`:** The companion native mobile application provides a dedicated `SENML_LAST` screen displaying an ordered list of sensor cards with custom labels, units, latest formatted values, and manual ordering.
4. **Third-Party Telemetry Integration (NASA Open MCT):** Operators need a standardized historical query and live streaming interface (`/history` and `/stream`) compatible with Open MCT telemetry adapters.
5. **Configuration & Customization:** Operators must be able to declare dashboard metrics in the station YAML configuration file, while retaining the ability to add, rename, reorder, or remove cards directly from the web interface with persistence.

## Decision

We introduce a complete, high-performance SenML telemetry subsystem in `dtn-hdy-ham`:

### 1. Ingestion Engine & Built-in `SenmlService`
- **Permanent Endpoint:** Registers `dtn://<node>/senml` on startup with Hardy BPA.
- **Auto-Detection:** Inspects incoming bundle payloads to detect CBOR or JSON formatting seamlessly.
- **RFC 8428 Resolution Pipeline:**
  - `metric_name = (bn || "") + (n || "")`
  - `unit = u || bu || ""`
  - `timestamp_ms = round(((bt || 0) + (t || 0)) * 1000)`. If relative ($t \le 2^{28}$), anchored to the bundle `creation_timestamp`.
  - Values extracted into `value REAL` (numeric) or `str_value TEXT` (strings `vs`, booleans `vb`, or raw data `vd`).
- **Cryptographic Security:** Integrates with `DirectionalKeyStore` to verify BPSec BIB HMAC signatures on incoming telemetry bundles, dropping invalid bundles when `VerifyPolicy::Strict` is enabled.
- **Idempotence:** Duplicate or out-of-order bundles are safely merged via SQLite `ON CONFLICT REPLACE`.

### 2. Dedicated SQLite Database (`senml.db`)
- Created in `storage.data_dir` (with in-memory fallback when `storage.in_memory == true`).
- Connection configured with `PRAGMA journal_mode = WAL;`, `PRAGMA synchronous = NORMAL;`, `PRAGMA foreign_keys = ON;`.
- **`metric_dictionary` Table:**
  ```sql
  CREATE TABLE IF NOT EXISTS metric_dictionary (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL UNIQUE,
      unit TEXT,
      custom_label TEXT,
      display_order INTEGER,
      is_visible INTEGER NOT NULL DEFAULT 0
  );
  ```
- **`telemetry_history` Table:**
  ```sql
  CREATE TABLE IF NOT EXISTS telemetry_history (
      metric_id INTEGER NOT NULL,
      timestamp INTEGER NOT NULL,
      value REAL,
      str_value TEXT,
      ingest_time INTEGER NOT NULL,
      PRIMARY KEY (metric_id, timestamp) ON CONFLICT REPLACE,
      FOREIGN KEY (metric_id) REFERENCES metric_dictionary (id)
  ) WITHOUT ROWID;

  CREATE INDEX IF NOT EXISTS idx_telemetry_history 
  ON telemetry_history (metric_id, timestamp ASC);
  ```
- **In-Memory Cache:** Maintains a `HashMap<String, i64>` in RAM mapping metric names to `metric_id` to eliminate redundant SELECT queries during high-throughput packet bursts.

### 3. REST & SSE Endpoints under `/api/v1/senml/*`
- **`GET /api/v1/senml/metrics`:** Lists all known metrics with dictionary metadata.
- **`GET /api/v1/senml/metrics/latest`:** Returns the latest measurement point for metrics, with optional filtering (`?id=...` or `?ids=...`).
- **`GET /api/v1/senml/history`:** Open MCT compatible time-range extraction (`?id=...&start=...&end=...`) returning `{ "utc": ..., "value": ... }` sorted chronologically (`ASC`).
- **`GET /api/v1/senml/stream`:** Server-Sent Events stream emitting `event: telemetry` with `{"id": "...", "utc": ..., "value": ..., "unit": "..."}`.
- **Dashboard Management:**
  - `POST /api/v1/senml/dashboard/cards`: Adds a metric to the active dashboard.
  - `PUT /api/v1/senml/dashboard/cards/{id}`: Updates custom alias/label or display order.
  - `DELETE /api/v1/senml/dashboard/cards/{id}`: Removes a card from the dashboard (setting `is_visible = 0` without destroying historical telemetry data).
  - `DELETE /api/v1/senml/dashboard/cards`: Clears all cards from the dashboard.

### 4. YAML Configuration & Dynamic Synchronization
- Declared in the station YAML file under the `senml:` block:
  ```yaml
  senml:
    metrics:
      - name: "urn:dtn:node1:power/bus_voltage"
        label: "Tension Bus"
      - name: "urn:dtn:node1:temp/ambient"
        label: "Température Ambiante"
  ```
- Loaded on daemon startup to initialize visible dashboard cards and aliases.
- Auto-discovered metrics from received bundles are persisted in `telemetry_history` and `metric_dictionary` with `is_visible = 0`.
- Operators can add discovered metrics to the dashboard or edit labels/order from the web interface, with changes synchronized back to the YAML file.

### 5. Frontend SenML Dashboard Tab
- Added to the main and mobile bottom navigation bars directly after "Files".
- Displays an ordered grid/list of cards matching `SENML_LAST`:
  - Custom label (or full metric name) with inline edit pencil.
  - Formatted measurement value with unit badge.
  - Relative time elapsed and absolute UTC timestamp tooltip.
  - Up/Down reordering arrows and remove button.
- Top action bar featuring "+ Ajouter une carte" (modal with discovered metric selection) and "Tout effacer".
- Reactive live value updates connected to the SSE telemetry stream.

## Consequences

### Positive
- Full feature parity with `dtn-android-messenger`'s `SENML_LAST` screen.
- First-class support for NASA Open MCT telemetry dashboards without external proxies.
- Zero data loss for auto-discovered metrics while keeping the operator dashboard clean and intentional.
- Delay-tolerant timestamp accuracy anchored to bundle creation time.
- Robust SQLite performance with WAL mode, indexation, and memory caching.

### Negative
- Managing both CBOR and JSON deserialization in Rust requires adding `ciborium` or leveraging `hardy_bpv7::cbor` alongside `serde_json`.
