# ADR 4: Design and Strategy for `dtnquery` Utility

## Status
Accepted

## Context
We need to implement `dtnquery`, a command-line inspection utility for the Hardy BPA that mirrors the behavior and subcommands of the `dtn7-rs` `dtnquery` tool. 

The subcommands of `dtnquery` are:
1. `nodeid` - Displays the local node identifier.
2. `eids` - Lists registered endpoint IDs.
3. `peers` - Lists known network peers.
4. `bundles` - Lists bundles currently held by the node.
5. `store` - Lists bundle status in store.
6. `info` - General daemon information.

Unlike `dtn7-rs`, which exposes a REST API to query all runtime daemon state, the Hardy BPA only exposes dynamic bidirectional gRPC stream interfaces (for services, applications, CLAs, and routing agents). No query APIs currently exist for in-memory states like registered endpoints or peer discovery.

## Decision
We will implement `dtnquery` using a hybrid approach combining configuration parsing, offline database access, and gRPC placeholders:

1. **Configuration Resolution**: `dtnquery` will load the Hardy daemon config (resolving from `--config`, `HARDY_BPA_SERVER_CONFIG_FILE` env var, or `/etc/hardy/bpa`) using the `config` crate. It will support TOML, YAML, and JSON configuration formats.
2. **Offline Commands**:
   - `nodeid`: Read directly from the parsed configuration file (`node-ids`).
   - `bundles` & `store`: Locate the storage metadata type (`storage.metadata.type`) from the Hardy configuration:
     - For `sqlite`: Resolve the SQLite file path (`db-dir` and `db-name`), open it via `rusqlite` (aligned with Hardy's `sqlite-storage` backend), and query the `bundles`, `unconfirmed_bundles`, and `waiting_queue` tables.
     - For `postgres`: Parse the connection string (`database-url`) and connect via `sqlx` (aligned with Hardy's `postgres-storage` backend) to query the same table structures.
     - *Note*: While both storage backends are supported, only the SQLite backend will be tested and verified locally due to environment constraints.
   - `bundles` formatting: We will parse raw bundle CBOR payloads from the database using `hardy_bpv7::bundle::ParsedBundle::parse_with_keys(data, &hardy_bpv7::bpsec::key::KeySet::EMPTY)`.
     - Output will format EIDs, creation timestamps, and sizes in the plaintext format `"source dest creation_time_milliseconds size"`.
     - Filter by `--addr` will check if either EID contains the filter pattern.
     - `--digest` will sort the bundle ID keys and hash them sequentially using `Sha1` to mimic `dtn7-rs`.
   - `store` formatting: We will map Hardy's `status_code` values (0 = New, 1 = Waiting, 2 = ForwardPending, 3 = AduFragment, 4 = Dispatching, 5 = WaitingForService) to readable status constraints, and print them as a JSON list of formatted strings `"id {constraints}"` to mirror `dtn7-rs`'s `bundles_status()` output.
   - `info`: Print a JSON object mirroring DTN7 `/status/info` containing stats (`incoming`, `dups`, `outgoing`, `delivered`, `broken`). The stats will be calculated from the database status counts:
     - `incoming` will count New (0), Waiting (1), and WaitingForService (5) bundles.
     - `outgoing` will count ForwardPending (2) and Dispatching (4) bundles.
     - `dups`, `delivered`, and `broken` will default to `0` (since Hardy does not track historical metrics in the database).
3. **Dynamic In-Memory Commands** (`eids` & `peers`):
   - These subcommands will be implemented in the CLI to preserve API compatibility.
   - However, since this data is kept purely in memory by the running BPA server and not exposed, these commands will output a placeholder message stating that the information is currently not available, referencing the required Hardy server enhancements.
4. **Proposed Hardy Enhancements**:
   - Create `TICKETS-FOR-HARDY.md` to document feature requests for Hardy to expose gRPC query APIs or persist dynamic state in SQLite.

## Consequences
- `dtnquery` can run offline (independent of the daemon process) to inspect stored bundles, configuration, and store state.
- Preserves the CLI contract of the `dtn7-rs` `dtnquery` utility.
- Identifies and formally tracks the gaps in Hardy's gRPC/storage layer via `TICKETS-FOR-HARDY.md` for future server-side enhancements.
