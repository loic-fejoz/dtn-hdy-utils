#!/usr/bin/env python3
"""
End-to-end integration test and verification script for the `dtnbasket` responder service.
Uses the workspace binary utilities `dtnsend` and `dtntrigger` to send a CBOR-serialized
request and verify the received CBOR response payload.
"""

import os
import sys
import time
import subprocess
import tempfile
import shutil
import re

# Try importing cbor2
try:
    import cbor2
except ImportError:
    print(
        "Error: The 'cbor2' python library is required to serialize and deserialize DTN Basket messages.\n"
        "Please install it using:\n"
        "    pip install cbor2\n",
        file=sys.stderr,
    )
    sys.exit(1)


def main():
    print("[*] Starting dtnbasket end-to-end integration test...")

    # Determine local directory paths
    script_dir = os.path.dirname(os.path.abspath(__file__))
    workspace_dir = os.path.dirname(script_dir)
    cargo_toml_path = os.path.join(workspace_dir, "Cargo.toml")

    # 1. Resolve local node EID by starting dtnprint for a second
    print("[*] Querying local Hardy instance for node EID...")
    dtnprint_proc = subprocess.Popen(
        ["cargo", "run", "--bin", "dtnprint", "--", "--service", "basket_detect_eid"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    local_node_eid = None
    time_limit = time.time() + 5.0
    while time.time() < time_limit:
        line = dtnprint_proc.stderr.readline()
        if not line:
            break
        # Match 'Listening for bundles on: dtn://<node_name>/basket_detect_eid'
        match = re.search(r"Listening for bundles on:\s*(dtn://[^/]+/)", line)
        if match:
            local_node_eid = match.group(1)
            break

    dtnprint_proc.terminate()
    dtnprint_proc.wait()

    if not local_node_eid:
        print(
            "[-] Error: Could not detect local node EID. Is Hardy BPA running on your system?\n"
            "    Please ensure your local Hardy instance is active and reachable via gRPC (port 50051).",
            file=sys.stderr,
        )
        sys.exit(1)

    print(f"[+] Detected local node EID: {local_node_eid}")

    # Define endpoints
    responder_service_name = "dtnbasket_test"
    reply_service_name = "basket_test_reply"
    responder_eid = f"{local_node_eid}{responder_service_name}"
    reply_eid = f"{local_node_eid}{reply_service_name}"

    # Temp directories/files for test runs
    temp_dir = tempfile.mkdtemp(prefix="dtnbasket_test_")
    config_path = os.path.join(temp_dir, "dtnbasket.toml")
    trigger_script_path = os.path.join(temp_dir, "capture_reply.py")
    request_cbor_path = os.path.join(temp_dir, "request.cbor")
    reply_cbor_path = os.path.join(temp_dir, "reply.cbor")

    try:
        # Load examples/dtnbasket.toml and customize the service name for test isolation
        with open("examples/dtnbasket.toml", "r") as f:
            toml_content = f.read()
        toml_content = toml_content.replace('service_name = "dtnbasket"', f'service_name = "{responder_service_name}"')
        with open(config_path, "w") as f:
            f.write(toml_content)

        # 3. Create the dtntrigger payload capture helper script
        with open(trigger_script_path, "w") as f:
            f.write(f"""import sys, shutil, cbor2
try:
    with open(sys.argv[2], "rb") as f:
        content = f.read()
    print(f"[trigger] Parsing file {{sys.argv[2]}} of size {{len(content)}} bytes", file=sys.stderr)
    data = cbor2.loads(content)
    if isinstance(data, cbor2.CBORTag):
        print(f"[trigger] Found CBOR Tag {{data.tag}}", file=sys.stderr)
        data = data.value
    print(f"[trigger] Decoded data type: {{type(data)}}, repr: {{repr(data)}}", file=sys.stderr)
    if isinstance(data, (dict, cbor2.frozendict)):
        print(f"[trigger] Decoded dict keys: {{list(data.keys())}}", file=sys.stderr)
        if 0 in data and 1 in data and 2 in data:
            print(f"[trigger] Matching BasketResponse! Copying to {reply_cbor_path}", file=sys.stderr)
            shutil.copy(sys.argv[2], "{reply_cbor_path}")
except Exception as e:
    print(f"[trigger] Error: {{e}}", file=sys.stderr)
""")

        # 4. Start the dtnbasket responder service in the background
        print("[*] Spawning dtnbasket service...")
        basket_proc = subprocess.Popen(
            ["cargo", "run", "--bin", "dtnbasket", "--", "-c", config_path, "-v"],
            stdout=subprocess.DEVNULL,
            stderr=sys.stderr,
            text=True,
        )

        # Wait a brief moment to let dtnbasket register
        time.sleep(1.5)
        if basket_proc.poll() is not None:
            print("[-] Error: dtnbasket exited immediately.", file=sys.stderr)
            sys.exit(1)

        # 5. Start dtntrigger to capture the reply bundle
        print("[*] Spawning dtntrigger to capture reply payloads...")
        trigger_proc = subprocess.Popen(
            [
                "cargo",
                "run",
                "--bin",
                "dtntrigger",
                "--",
                "-e",
                reply_service_name,
                "-c",
                f"python3 {trigger_script_path}",
                "-v",
            ],
            stdout=subprocess.DEVNULL,
            stderr=sys.stderr,
        )

        time.sleep(1.0)
        if trigger_proc.poll() is not None:
            print("[-] Error: dtntrigger exited immediately.", file=sys.stderr)
            sys.exit(1)

        # 6. Send the request bundle using dtnbasket-cli (SEARCH and LIST)
        print(f"[*] Sending BasketRequest using dtnbasket-cli to {responder_eid}...")
        send_proc = subprocess.run(
            [
                "cargo",
                "run",
                "--bin",
                "dtnbasket-cli",
                "--",
                "--sender",
                reply_service_name,
                "--receiver",
                responder_eid,
                "--reply-to",
                reply_service_name,
                "--req-id",
                "req-test-123",
                "--search",
                "dtnbasket",
                "--list",
                "stats",
            ],
            capture_output=True,
            text=True,
        )
        print(f"[+] dtnbasket-cli stdout:\n{send_proc.stdout}")
        print(f"[+] dtnbasket-cli stderr:\n{send_proc.stderr}")
        if send_proc.returncode != 0:
            sys.exit(1)

        # 8. Poll for captured reply
        print("[*] Waiting for reply bundle to be routed and captured...")
        reply_received = False
        timeout = time.time() + 10.0
        while time.time() < timeout:
            if os.path.exists(reply_cbor_path):
                reply_received = True
                break
            time.sleep(0.5)

        if not reply_received:
            print("[-] Error: Request timed out. No response captured.", file=sys.stderr)
            sys.exit(1)

        # 9. Decode and verify the BasketResponse CBOR
        print("[*] Decoding captured response CBOR...")
        with open(reply_cbor_path, "rb") as f:
            raw_reply = f.read()

        try:
            decoded_reply = cbor2.loads(raw_reply)
        except Exception as e:
            print(f"[-] Error parsing response CBOR: {e}", file=sys.stderr)
            sys.exit(1)

        # Handle potential outer tag
        if isinstance(decoded_reply, cbor2.CBORTag):
            print(f"[+] Found outer tag: {decoded_reply.tag}")
            decoded_reply = decoded_reply.value

        print(f"[+] Decoded BasketResponse: {decoded_reply}")

        # Assertions
        assert decoded_reply.get(0) == 1, "Incorrect version field"
        assert decoded_reply.get(1) == "req-test-123", "Incorrect req_id field"

        items = decoded_reply.get(2, [])
        assert len(items) == 3, f"Incorrect number of response items: expected 3, got {len(items)}"

        # Search dtnbasket results (indices 0 and 1)
        search_res_0 = items[0]
        search_res_1 = items[1]
        print(f"[+] Search result 0: {search_res_0}")
        print(f"[+] Search result 1: {search_res_1}")

        assert search_res_0.get(0) == 0, "Incorrect item index for search result 0"
        assert search_res_0.get(1) == 69, f"Unexpected CoAP status: {search_res_0.get(1)}"
        assert search_res_1.get(0) == 0, "Incorrect item index for search result 1"
        assert search_res_1.get(1) == 69, f"Unexpected CoAP status: {search_res_1.get(1)}"

        # Verify URIs
        uris = [search_res_0.get(2, {}).get(3), search_res_1.get(2, {}).get(3)]
        assert any("dtnbasket.rs" in u for u in uris if u), "dtnbasket.rs not found in search results"
        assert any("dtnbasket-cli.rs" in u for u in uris if u), "dtnbasket-cli.rs not found in search results"

        # List stats results (index 2)
        list_res = items[2]
        print(f"[+] List result: {list_res}")
        assert list_res.get(0) == 1, "Incorrect item index for list result"
        assert list_res.get(1) == 69, f"Unexpected CoAP status: {list_res.get(1)}"
        list_uri = list_res.get(2, {}).get(3)
        assert list_uri == "stats", f"Unexpected URI for list result: {list_uri}"
        list_mime = list_res.get(2, {}).get(2)
        assert list_mime == "text/markdown; charset=utf-8", f"Unexpected MIME type for list result: {list_mime}"

        # Clean up processes
        print("[+] E2E Test completed successfully!")

    finally:
        # Tear down background services
        print("[*] Cleaning up background processes and temporary directories...")
        try:
            basket_proc.terminate()
            basket_proc.wait()
        except Exception:
            pass

        try:
            trigger_proc.terminate()
            trigger_proc.wait()
        except Exception:
            pass

        shutil.rmtree(temp_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
