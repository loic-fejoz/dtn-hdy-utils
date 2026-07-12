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
