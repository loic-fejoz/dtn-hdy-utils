# DTN `hardy` utility tools (dtn-hdy-utils)

A collection of utility tools to interact with the `hardy` BPA implementation of Bundle Protocol version 7 (BPv7 / RFC 9171) Delay Tolerant Network (DTN).

## Utilities Included

- [**`dtnprint`**](src/bin/dtnprint.rs): Subscribes to the BPA and prints received textual bundle payloads directly to `stdout`.
- [**`dtnsend`**](src/bin/dtnsend.rs): Sends bundle payloads to a specified receiver EID via gRPC, matching `dtn7` command-line flags and supporting dry-run hex generation.
- [**`dtnquery`**](src/bin/dtnquery.rs): Directly queries local metadata databases (SQLite/PostgreSQL , until `hardy` is providing such query/management interfaces) to inspect the node's bundle store state offline, matching the subcommand and stats formatting of the `dtn7-rs` `dtnquery` tool.
- [**`dtntrigger`**](src/bin/dtntrigger.rs): Subscribes to a specific DTN service endpoint and either prints incoming payloads directly or executes a shell command with the payload written to a temporary file, matching the behavior of the `dtn7-rs` `dtntrigger` utility.
- [**`dtnping`**](src/bin/dtnping.rs): Connects to a running local Hardy instance, registers as an application, sends ping bundles to a destination EID, and measures round-trip times (RTT) and path hops.
  *(Difference from Hardy's built-in `bp ping`: Hardy's built-in utility runs an entire standalone BPA daemon inline and establishes a direct Convergence Layer connection (e.g. TCPCLv4) to the destination. Conversely, `dtnping` registers purely as a lightweight application layer client on a local running Hardy daemon over gRPC, sending bundles through it).*
- [**`hdy-stats`**](src/bin/hdy-stats.rs): Connects to the local Hardy BPA via gRPC, monitors the BPA's log output (either through a log file or systemd journald), records incoming/outgoing bundle traffic in a local SQLite database, and acts as a DTN service responder to answer statistics queries with formatted text reports.
- [**`dtnbasket`**](src/bin/dtnbasket.rs): A responder service/application for Hardy DTN BPv7 implementing the DTN Basket Protocol ([draft-f4jxq-dtn-basket-00](draft-f4jxq-dtn-basket-00.xml)). It acts as a delay-tolerant resource proxy that serves local mapped paths, directories (with regex file matching), and proxy-fetches remote internet resources via HTTP(S) and Gemini protocols.
- [**`dtnbasket-cli`**](src/bin/dtnbasket-cli.rs): A client CLI tool used to forge, sign, and transmit CBOR-serialized Basket request bundles to a remote responder service.

See [examples](examples/README.md) for usage examples.

## DTN Basket Protocol (`dtnbasket` & `dtnbasket-cli`)

`dtnbasket` implements [draft-f4jxq-dtn-basket-00](draft-f4jxq-dtn-basket-00.xml) as a Rust service for Hardy DTN BPv7. It acts as an offline proxy that receives resource request lists (`BasketRequest`) packaged as CBOR, resolves them, and returns them to the caller (`BasketResponse`).

### Features
*   **Protocols Supported**: Fetches remote resources over `http://`, `https://`, and `gemini://` protocols.
*   **Local File Server**: Serves files within directories listed in the configuration `allowed_dirs`.
*   **URN Mappings**: Resolves specific static URN paths (e.g. `urn:dtn:doc:yaesu-ft817-um-fr`) to local file paths.
*   **Search/List**: Resolves directory LIST and SEARCH queries using filename regex.
*   **Security & Safety**: Integrates SSRF protection blocking queries to loopback/private network addresses, implements directory traversal protection, and validates BPSec bundle signatures.

### Configuration

The responder is configured using a flat TOML file. An example is provided at [`examples/dtnbasket.toml`](examples/dtnbasket.toml):

```toml
service_name = "dtnbasket"
insecure_tls = false
allowed_dirs = ["/home/loic/quickref"]

[mappings]
"urn:dtn:doc:yaesu-ft817-um-fr" = "/home/loic/quickref/FT-817_user_FR.pdf"
```

Start the responder:
```bash
cargo run --bin dtnbasket -- -c examples/dtnbasket.toml -v
```

### Sending Requests

Use `dtnbasket-cli` to construct and dispatch requests:
```bash
cargo run --bin dtnbasket-cli -- \
    --receiver dtn://node2/dtnbasket \
    --get urn:dtn:doc:yaesu-ft817-um-fr \
    --get https://w.fejoz.net
```

Refer to [AGENTS.md](AGENTS.md) for build, testing, and formatting guidelines.

## Ideas

Below is a prioritized list of potential utility tools to implement as DTN applications for this project, including data format (MIME type) details and ham radio integrations:

  - *Data format*: Typically `text/plain` (raw commands/scripts) or `application/json` (structured parameters).
- [ ] **`dtnaprs` (APRS Gateway)**: A bridge that parses local AX.25 APRS (Automatic Packet Reporting System) packets (telemetry, weather, positions) from a radio TNC and forwards them.
  - *Data format*: `text/plain` containing standard APRS text string representations, or `application/vnd.aprs` for structured APRS packets.
- [ ] **`dtnrss` (RSS/Atom Relayer)**: A tool that periodically fetches remote RSS or Atom feeds, converts new articles into offline digests, and distributes them via DTN.
  - *Data format*: `application/rss+xml`, `application/atom+xml`, or pre-rendered `text/markdown` digests.
- [ ] **`dtnmqtt`**: An MQTT-to-DTN bridge that subscribes to local MQTT broker topics, packages message payloads into DTN bundles, and publishes received DTN bundles back to MQTT.
  - *Data format*: A custom envelope (`application/json` or binary `application/x-msgpack`) wrapping the destination topic and payload.
- [ ] **`dtnbbs` / `dtnbulletin` (Ham Bulletin BBS)**: A modern digital BBS relayer that uses DTN bundle multicast groups to exchange and synchronize local bulletins and traffic reports.
  - *Data format*: Standard amateur BBS email-like text formats (`message/rfc822` or `text/plain`).
- [ ] **`dtncp` / `dtnfile`**: A utility to copy files or directories over DTN. It splits large files and handles payload reassembly.
  - *Data format*: Multiplexed chunks containing the target file payload (`application/octet-stream`) accompanied by transfer metadata (`application/json`).
- [ ] **`dtnqsl` (Delay-Tolerant QSL Exchange)**: A utility that automatically formats, signs, and queues digital QSL contact cards as DTN bundles.
  - *Data format*: ADIF (Amateur Data Interchange Format) payload wrapped as `text/plain`, or structured `application/json`.
- [ ] **`dtnchat`**: A simple terminal chat client allowing interactive messaging between two or more DTN nodes.
  - *Data format*: Plain text messages (`text/plain`).
- [ ] **`dtnwinlink` (Winlink Email Gateway)**: An email-to-DTN gateway that wraps Winlink amateur radio email traffic into BPv7 bundles.
  - *Data format*: Standard Internet Message email format (`message/rfc822`).
- [ ] **`dtnfediverse` / `dtnmastodon`**: A gateway that queues and relays microblog posts (like Mastodon statuses) over DTN links.
  - *Data format*: JSON-LD ActivityPub activities (`application/activity+json`).
- [ ] **`dtndiscord` / `dtnslack`**: A bot connector that acts as a gateway to queue system alerts or textual notifications locally, posting them directly to webhooks when online.
  - *Data format*: `application/json` matching the specific Discord or Slack webhook payload schemas.
- [ ] **`dtnbpq` (Bundle Protocol Query)**: A content-centric DTN caching and discovery tool. It implements a query/response mechanism (based on `draft-irtf-dtnrg-bpq-00`) allowing nodes to query intermediate caches for specific resources or files, enabling in-network caching and local resource resolution without contacting the origin server directly.
  - *Data format*: Structured query and response envelopes encoded as `application/json` or CBOR-based binary schemas.
