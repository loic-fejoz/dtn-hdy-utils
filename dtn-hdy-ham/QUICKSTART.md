# Quickstart Guide — `dtn-hdy-ham`

Welcome to **dtn-hdy-ham**, the Delay-Tolerant Networking (DTN) BPv7 Bundle Processing Agent tailored for amateur radio operators and field communication.

This release package contains static, self-contained binaries that require no external runtime dependencies.

---

## 1. Package Contents

- `bin/`:
  - `dtn-hdy-ham`: Main DTN BPv7 node, Web UI server, and convergence layer engine.
  - `dtnsend`: Command-line bundle sending utility.
  - `dtnprint`: Command-line bundle receiving / logging utility.
  - `dtnfiles`: Command-line file distribution and retrieval tool.
  - `dtntrigger`: Command-line event execution utility on bundle reception.
- `config.example.toml`: Documented configuration template.
- `dtn-hdy-ham.service`: Systemd service unit template for Linux systems.

---

## 2. Fast Setup (30 Seconds)

### Step 2.1 — Configuration
You can either run the interactive setup wizard:
```bash
./bin/dtn-hdy-ham init
```
Or copy the reference configuration and customize your callsign:
```bash
cp config.example.toml config.toml
# Edit config.toml with your favorite editor (e.g. nano config.toml)
```

### Step 2.2 — Start the Node
```bash
./bin/dtn-hdy-ham -c config.toml
```

### Step 2.3 — Open the Web Dashboard
Open your web browser and navigate to:
```
http://127.0.0.1:8080
```
From the Web Dashboard, you can:
- **Chat**: Send and receive DTN messages in real-time with other stations.
- **Files**: Share files, inspect incoming payloads, and mark files as *Sticky*.
- **Bundles & Stats**: Inspect live bundle queues, active convergence layers, and peers.

---

## 3. Using the Bundled CLI Tools

All bundled CLI tools connect automatically to the local `dtn-hdy-ham` gRPC interface (port 50051).

### Send a Text Message or File (`dtnsend`)
```bash
# Send a simple text string:
./bin/dtnsend -r "dtn://peer.ampr.org/incoming" "Hello over DTN!"

# Send the contents of a file:
./bin/dtnsend -r "dtn://peer.ampr.org/incoming" /path/to/document.txt
```

### Listen for Incoming Bundles (`dtnprint`)
```bash
# Listen on endpoint 'incoming' and print received bundle payloads to stdout:
./bin/dtnprint -e incoming
```

### Distribute and Fetch Files (`dtnfiles`)
```bash
# Broadcast or send a file to a remote peer:
./bin/dtnfiles send -r "dtn://peer.ampr.org/" -f telemetry.log

# List files published by a remote node:
./bin/dtnfiles list -r "dtn://peer.ampr.org/"
```

### Execute Scripts on Bundle Reception (`dtntrigger`)
```bash
# Run a custom shell command whenever a bundle arrives at 'dtn://<node>/alert':
./bin/dtntrigger -e alert -c "logger Received emergency DTN alert"
```

---

## 4. Running as a Background Systemd Service (Linux)

To run `dtn-hdy-ham` continuously on Linux (e.g. Raspberry Pi or server):

1. Copy the binaries to your path (or keep them in `/opt/dtn-hdy-ham/bin`):
   ```bash
   sudo cp bin/* /usr/local/bin/
   ```

2. Copy and customize configuration:
   ```bash
   sudo mkdir -p /etc/dtn-hdy-ham
   sudo cp config.example.toml /etc/dtn-hdy-ham/config.toml
   sudo nano /etc/dtn-hdy-ham/config.toml
   ```

3. Install the systemd unit:
   ```bash
   sudo cp dtn-hdy-ham.service /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl enable --now dtn-hdy-ham
   ```

4. View logs:
   ```bash
   journalctl -u dtn-hdy-ham -f
   ```
