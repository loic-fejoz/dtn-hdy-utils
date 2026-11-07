# Examples

This directory contains example scripts and configurations for the `dtn-hdy-utils` tools.

## Desktop Notification Trigger (`dtn-notify.sh`)

The `dtn-notify.sh` script is a helper for `dtntrigger` that displays a desktop notification when a bundle is received.

### How to use:

1. Make sure the script is executable:
   ```bash
   chmod +x examples/dtn-notify.sh
   ```

2. Run `dtntrigger` with this script as the command, specifying the service endpoint (e.g. `ntfy`):
   ```bash
   cargo run --release --bin dtntrigger -- -p 50051 -e ntfy -c ./examples/dtn-notify.sh -v
   ```

3. Send a test bundle via `dtnsend` to the endpoint:
   ```bash
   cargo run --release --bin dtnsend -- -p 50051 -s dtn://f4jxq/incoming dtn://f4jxq/ntfy "Hello from DTN!"
   ```

4. You will receive a desktop notification showing:
   - **Summary**: `DTN Bundle from dtn://f4jxq/incoming`
   - **Body**: `Hello from DTN!`

## Matrix Notification Trigger (`dtn-matrix.py`)

The `dtn-matrix.py` script is a helper for `dtntrigger` that forwards received bundle payloads to a specified Matrix room. It fully supports both unencrypted and end-to-end encrypted (E2EE) rooms.

### How to use:

1. Install the `matrix-nio` Python library with encryption (E2EE) support:
   ```bash
   # Note: If your system uses CMake 4.0+, set the compatibility policy env var to compile python-olm successfully
   CMAKE_POLICY_VERSION_MINIMUM=3.5 pip install "matrix-nio[e2e]"
   ```

2. Copy the template configuration file:
   ```bash
   cp examples/matrix_config.json.example examples/matrix_config.json
   ```

3. Populate `examples/matrix_config.json` with your Matrix homeserver, credentials (user ID, device ID, stored access token), target room ID, and optionally `"store_path"` to override the location of the local cryptographic keys database.
   
   *Tip: You can optionally add `"password": "your_password"` to the configuration. If the access token expires or becomes inactive, the script will automatically log in with the password, fetch a new access token, update the configuration file on disk, and retry the send operation.*

4. Make the script executable:
   ```bash
   chmod +x examples/dtn-matrix.py
   ```

5. Run `dtntrigger` specifying the `matrix` service endpoint:
   ```bash
   cargo run --release --bin dtntrigger -- -p 50051 -e matrix -c ./examples/dtn-matrix.py -v
   ```

6. Send a test bundle via `dtnsend` to the endpoint:
   ```bash
   cargo run --release --bin dtnsend -- -p 50051 -s dtn://f4jxq/incoming dtn://f4jxq/matrix "Matrix alert via DTN!"
   ```

*Note: For E2EE rooms, the script automatically initializes a local keys database store (`matrix_store`), syncs room member keys, and uses `ignore_unverified_devices=True` to route encrypted alerts seamlessly.*

## DTN Traffic Statistics Service (`hdy-stats`)

The `hdy-stats` utility records traffic volume (the number of unique bundles routed) and processes real-time stats queries over the DTN network.

### How to use:

1. **Query stats remotely over the DTN network**:
   You can query the stats service of any node on the network where the app is installed.
   
   a. Start the `dtnprint` listener on your local node to capture the response on a dedicated endpoint (e.g. `myreply`):
      ```bash
      cargo run --release --bin dtnprint -- --service myreply
      ```

   b. Send an empty bundle to the stats service of the target node (e.g. `dtn://f4jxq/stats`) setting the sender to your reply endpoint:
      ```bash
      cargo run --release --bin dtnsend -- --sender myreply --receiver dtn://f4jxq/stats ""
      ```

   c. The listener will print the stats report sent back by the stats service:
      ```text
      From: dtn://f4jxq/stats
      --- 24h ---
      dtn://f4jxq/: 35 bundle(s)
      dtn://f4jxq/echo: 7 bundle(s)
      dtn://f4jxq-2/myreply: 3 bundle(s)
      ```

### How to test locally:

1. **Start the service in the background** (monitors the BPA logs and listens on `dtn://<node>/stats`):

   *Using journalctl (requires the user to belong to the `systemd-journal` group):*
   ```bash
   cargo run --release --bin hdy-stats -- --journald-unit hardy-bpa -v
   ```

   *Using a dedicated log file:*
   ```bash
   cargo run --release --bin hdy-stats -- --log-file /var/log/hardy/hardy.log -v
   ```

2. **Generate some test traffic** (e.g. by pinging or sending messages):
   ```bash
   cargo run --release --bin dtnping -- dtn://f4jxq/echo -c 2
   ```

3. **Query stats locally** (prints a formatted text report of the traffic ranking from the SQLite database):
   ```bash
   cargo run --release --bin hdy-stats -- --show
   ```

## DTN Fortune Teller Responder (`dtn-fortune.sh`)

The `dtn-fortune.sh` script is a helper for `dtntrigger` that acts as an automated fortune teller responder over DTN. When queried, it randomly selects a DTN/networking-themed fortune cookie (in English or French) from custom fortune databases and transmits the response back to the requester via `dtnsend`.

### Requirements:

Ensure `fortune` and `strfile` are installed on the host system:
```bash
sudo apt install fortune-mod
```

### How to use:

1. Make sure the script is executable:
   ```bash
   chmod +x examples/dtn-fortune.sh
   ```

2. Run `dtntrigger` listening on the `fortune` service endpoint:
   ```bash
   cargo run --release --bin dtntrigger -- -p 50051 -e fortune -c ./examples/dtn-fortune.sh -v
   ```

3. Start `dtnprint` on the client node to receive and display the fortune response (e.g. on endpoint `myfortune`):
   ```bash
   cargo run --release --bin dtnprint -- --service myfortune
   ```

4. Request a fortune via `dtnsend`:
   
   - **English fortune (default / empty payload)**:
     ```bash
     echo "" | cargo run --release --bin dtnsend -- -s myfortune -r dtn://<target-node>/fortune
     ```

   - **French fortune (payload `fr` or `french`)**:
     ```bash
     echo "fr" | cargo run --release --bin dtnsend -- -s myfortune -r dtn://<target-node>/fortune
     ```

5. The client `dtnprint` listener will display the returned fortune:
   ```text
   From: dtn://<target-node>/fortune
   In DTN, the fastest route is patience.
   ```

## Ecowitt Weather Station SenML Collector (`dtn-ecowitt.py`)

The `dtn-ecowitt.py` script queries real-time weather data from the Ecowitt HTTP API, converts outdoor/indoor temperature (from °F to °C) and humidity into a standard SenML CBOR payload (RFC 8428), and sends it to a target DTN receiver (e.g. `dtn://N0CALL/senml`) using `dtnsend`.

### How to use:

1. Ensure the script is executable:
   ```bash
   chmod +x examples/dtn-ecowitt.py
   ```

2. Fetch from Ecowitt API and send directly over DTN:
   ```bash
   ./examples/dtn-ecowitt.py
   ```

3. Dry-run inspection (prints the SenML JSON structure and CBOR hex payload):
   ```bash
   ./examples/dtn-ecowitt.py -D
   ```

4. Options and overrides:
   ```bash
   ./examples/dtn-ecowitt.py \
       --app-key "<APPLICATION_KEY>" \
       --api-key "<API_KEY>" \
       --mac "AA:BB:CC:DD:EE:FF" \
       --receiver "dtn://N0CALL/senml" \
       --base-urn "urn:ecowitt:aa-bb-cc-dd-ee-ff:" \
       --lifetime 7200
   ```

## Two-Station Hamradio BPA Test (`stata.yaml` & `statb.yaml`)

This demonstrates two `dtn-hdy-ham` instances interconnected via TCPCLv4:
- `STATA` (`examples/stata.yaml`): Node EID `dtn://stata.ampr.org/`, gRPC port `3050`, TCPCL port `4551`, connecting to STATB on `127.0.0.1:4552`.
- `STATB` (`examples/statb.yaml`): Node EID `dtn://statb.ampr.org/`, gRPC port `3051`, TCPCL listening on `127.0.0.1:4552`.

### Running the demonstration:

1. Start STATB in one terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/statb.yaml
   ```

2. Start STATA in a second terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/stata.yaml
   ```

3. Listen for bundles on STATB using `dtnprint`:
   ```bash
   cargo run --bin dtnprint -- --port 3051 --service incoming --keystore /dev/null -v
   ```

4. Transmit a bundle from STATA to STATB using `dtnsend`:
   ```bash
   echo "Hello STATB from STATA via TCPCL!" | cargo run --bin dtnsend -- --port 3050 --receiver dtn://statb.ampr.org/incoming -v
   ```

## Two-Station Hamradio LTP over Mercury TNC (`stata-ltp.yaml` & `statb-ltp.yaml`)

This demonstrates two `dtn-hdy-ham` instances interconnected via the LTP Convergence Layer Adapter (`hardy-ltp-cla`) using Mercury TNC KISS TCP interfaces over simulated audio channels (`launch_dual_mercury.sh`):
- **Mercury Bridge**: Runs two Mercury TNC instances linked by an 8 kHz real-time audio FIFO bridge with shared simplex RF channel simulation (`simplex_audio_bridge.py`). Station A exposes KISS TCP on `127.0.0.1:8100`, Station B on `127.0.0.1:8500`.
- **Flow Control & Pacing**: Demonstrates Leaky-Bucket aggregate transmission rate limiting (`rate-limit-bytes-per-sec: 1200`) and Half-Duplex Checkpoint turnaround pacing (`half-duplex: true`, `turnaround-mode: wait-for-report`).
- `STATA` (`examples/stata-ltp.yaml`): Node EID `dtn://stata.ampr.org/`, gRPC port `3050`, LTP Engine ID `1`, KISS TCP client connected to `127.0.0.1:8100`, peer Engine ID `2` (`dtn://statb.ampr.org/`).
- `STATB` (`examples/statb-ltp.yaml`): Node EID `dtn://statb.ampr.org/`, gRPC port `3051`, LTP Engine ID `2`, KISS TCP client connected to `127.0.0.1:8500`, peer Engine ID `1` (`dtn://stata.ampr.org/`).

### Automated Simplex Test:

To execute an end-to-end automated transfer across the simulated half-duplex RF channel:
```bash
./examples/test_simplex_mercury_ltp.sh
```

### Running the interactive demonstration:

1. Launch the dual Mercury TNC bridge (from `examples/launch_dual_mercury.sh` or the `mercury` repository):
   ```bash
   ./examples/launch_dual_mercury.sh
   ```

2. Start STATB in another terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/statb-ltp.yaml
   ```

3. Start STATA in a third terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/stata-ltp.yaml
   ```

4. Listen for bundles on STATB using `dtnprint`:
   ```bash
   cargo run --bin dtnprint -- --port 3051 --service incoming --keystore /dev/null -v
   ```

5. Transmit a bundle from STATA to STATB using `dtnsend`:
   ```bash
   echo "Hello STATB from STATA via LTP over Mercury KISS TNC!" | cargo run --bin dtnsend -- --port 3050 --receiver dtn://statb.ampr.org/incoming -v
   ```

## Two-Station Hamradio Mercury ARQ Connected Mode (`stata-mercury.yaml` & `statb-mercury.yaml`)

This demonstrates two `dtn-hdy-ham` instances interconnected via the Mercury ARQ Connected Convergence Layer (`hardy-mercury-cla`) and the scheduled `ContactCoordinator`:
- **Mercury Bridge**: `launch_dual_mercury.sh` runs Station A (Control port 8300, Data port 8301) and Station B (Control port 8400, Data port 8401) linked across an 8 kHz acoustic simulation bridge.
- **RADIANT Defaulting**: Specifying `callsign: "STATA"` automatically sets the node EID to `dtn://stata/` and configures the default CLA callsign. Specifying peer `callsign: "STATB"` automatically defaults the peer EID to `dtn://statb/`.
- **Scheduled Dialing**: `stata-mercury.yaml` configures a recurring contact plan (`cron: "*/10 * * * * *"`, `duration: "60s"`) that automatically dials STATB when the window opens and tears down when expired.

### Running the demonstration:

1. Launch the dual Mercury TNC bridge (from `examples/launch_dual_mercury.sh` or the `mercury` repository):
   ```bash
   ./examples/launch_dual_mercury.sh
   ```

2. Start STATB in another terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/statb-mercury.yaml
   ```

3. Start `dtnprint` on STATB:
   ```bash
   cargo run --bin dtnprint -- --port 3051 --service incoming --keystore /dev/null -v
   ```

4. Start `dtnprint` on STATA:
   ```bash
   cargo run --bin dtnprint -- --port 3050 --service incoming --keystore /dev/null -v
   ```

5. Start STATA in another terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/stata-mercury.yaml
   ```

6. When the scheduled contact opens (within seconds), send a bundle from STATA to STATB:
   ```bash
   echo "Hello STATB from STATA via Mercury ARQ Connected Mode!" | cargo run --bin dtnsend -- --port 3050 --receiver dtn://statb/incoming -v
   ```

7. Send a reply bundle back from STATB to STATA over the same active session (bidirectional):
   ```bash
   echo "Hello STATA from STATB in reverse direction!" | cargo run --bin dtnsend -- --port 3051 --receiver dtn://stata/incoming -v
   ```

## Web Dashboard and REST API (`test-web.yaml`)

This demonstrates the Axum web server and Preact single-page application embedded in `dtn-hdy-ham`:
- Exposes a REST API under `/api/v1/` (`GET /api/v1/config`, `POST /api/v1/bundles`).
- Serves the responsive Vite + Preact + Tailwind CSS web dashboard from `dtn-hdy-ham/web/dist` with client-side SPA routing fallback.
- Read-only visualization of the active station configuration, CLAs, peers, and static routes.
- Generic bundle sender supporting text messages and binary file uploads (`multipart/form-data`).

### Running the web dashboard:

1. Build the web assets (if modifying the frontend in `dtn-hdy-ham/web/`):
   ```bash
   cd dtn-hdy-ham/web && pnpm build && cd ../..
   ```

2. Start the `dtn-hdy-ham` daemon:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/test-web.yaml
   ```

3. Open your browser at `http://127.0.0.1:8088/` to access the dashboard.

## Two-Station Hamradio LTP over Direwolf G3RUH (`stata-dw-ltp.yaml` & `statb-dw-ltp.yaml`)

This demonstrates bidirectional BPv7 bundle transfer over the Licklider Transmission Protocol (RFC 5326) using Direwolf software TNCs connected via PipeWire/PulseAudio crossed null-sinks in G3RUH 9600 baud packet radio mode:
- **Direwolf Crossed Bridge**: `launch_dual_direwolf.sh` launches two Direwolf instances connected via PipeWire/PulseAudio null-sinks (`dw_a_tx` and `dw_b_tx` at 48000 Hz) using ALSA virtual PCM mapping. Both instances use `MODEM 9600` (G3RUH) and basic KISS TCP mode (Station A on port 8001, Station B on port 8101) with AX.25 and FX.25 disabled.
- **Compatibility**: `hardy-ltp-cla` supports an optional `min-frame-len` setting on `kiss-tcp` (defaulting to 0 for modems like Mercury with no padding). When set to `min-frame-len: 15` for Direwolf, it automatically formats small LTP control segments (such as Report Acknowledgment and Cancel frames) using standard RFC 5326 trailer extensions to meet TNC minimum frame size constraints (15 octets).
- `STATA` (`examples/stata-dw-ltp.yaml`): Node EID `dtn://stata.ampr.org/`, Web API `http://127.0.0.1:8081`, LTP Engine ID `1`, KISS TCP client connected to `127.0.0.1:8001`.
- `STATB` (`examples/statb-dw-ltp.yaml`): Node EID `dtn://statb.ampr.org/`, Web API `http://127.0.0.1:8082`, LTP Engine ID `2`, KISS TCP client connected to `127.0.0.1:8101`.

### Automated Bidirectional Test:

Run the automated test script which sets up Direwolf, launches both BPAs, transmits bundles in both directions via the REST API, and verifies reception in the SQLite and disk stores:
```bash
./examples/test_bidirectional_direwolf.sh
```

### Manual Demonstration:

1. Launch the dual Direwolf simulator:
   ```bash
   ./examples/launch_dual_direwolf.sh
   ```

2. Start STATB in another terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/statb-dw-ltp.yaml
   ```

3. Start STATA in a third terminal:
   ```bash
   cargo run -p dtn-hdy-ham -- --config examples/stata-dw-ltp.yaml
   ```

4. Transmit a bundle from STATA to STATB:
   ```bash
   curl -X POST http://127.0.0.1:8081/api/v1/bundles \
     -H "Content-Type: application/json" \
     -d '{"destination": "dtn://statb.ampr.org/chat", "payload": "Hello from STATA via Direwolf!"}'
   ```

5. Transmit a reply bundle from STATB back to STATA:
   ```bash
   curl -X POST http://127.0.0.1:8082/api/v1/bundles \
     -H "Content-Type: application/json" \
     -d '{"destination": "dtn://stata.ampr.org/chat", "payload": "Hello back from STATB via Direwolf!"}'
   ```




