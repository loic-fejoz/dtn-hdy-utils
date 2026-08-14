#!/usr/bin/env python3
"""
End-to-end integration test and verification script for the `dtnforward` BIBE utility.
Uses the workspace binary utilities `dtnsend`, `dtnforward`, and `dtntrigger` to verify
that a bundle received on one service is wrapped in a BIBE outer bundle and successfully
forwarded to the target destination.
"""

import os
import sys
import time
import subprocess
import tempfile
import shutil
import re

# Try importing cbor2 for CBOR validation
try:
    import cbor2
except ImportError:
    print(
        "Error: The 'cbor2' python library is required to parse and validate CBOR messages.\n"
        "Please install it using:\n"
        "    pip install cbor2\n",
        file=sys.stderr,
    )
    sys.exit(1)


def main():
    print("[*] Starting dtnforward end-to-end integration test...")

    # Determine local directory paths
    script_dir = os.path.dirname(os.path.abspath(__file__))
    workspace_dir = os.path.dirname(script_dir)

    # 1. Resolve local node EID by starting dtnprint for a second
    print("[*] Querying local Hardy instance for node EID...")
    dtnprint_proc = subprocess.Popen(
        ["cargo", "run", "--bin", "dtnprint", "--", "--service", "forward_detect_eid"],
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
        # Match 'Listening for bundles on: dtn://<node_name>/forward_detect_eid'
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
    forward_service_name = "dtnforwardtest"
    target_service_name = "dtnforwardtarget"
    forward_eid = f"{local_node_eid}{forward_service_name}"
    target_eid = f"{local_node_eid}{target_service_name}"

    # Temp directories/files for test runs
    temp_dir = tempfile.mkdtemp(prefix="dtnforward_test_")
    trigger_script_path = os.path.join(temp_dir, "capture_forward.py")
    captured_payload_path = os.path.join(temp_dir, "captured_inner.bin")

    forward_proc = None
    trigger_proc = None

    try:
        # 2. Create the dtntrigger payload capture helper script
        with open(trigger_script_path, "w") as f:
            f.write(f"""import sys, shutil
try:
    print(f"[trigger] Copying {{sys.argv[2]}} to {captured_payload_path}", file=sys.stderr)
    shutil.copy(sys.argv[2], "{captured_payload_path}")
except Exception as e:
    print(f"[trigger] Error: {{e}}", file=sys.stderr)
""")

        # 3. Start the dtnforward service in the background
        print(f"[*] Spawning dtnforward: {forward_eid} -> {target_eid}...")
        forward_proc = subprocess.Popen(
            [
                "cargo",
                "run",
                "--bin",
                "dtnforward",
                "--",
                "--service",
                forward_service_name,
                "--target",
                target_eid,
                "-v",
            ],
            stdout=subprocess.DEVNULL,
            stderr=sys.stderr,
            text=True,
        )

        # Wait a brief moment to let dtnforward register
        time.sleep(1.5)
        if forward_proc.poll() is not None:
            print("[-] Error: dtnforward exited immediately.", file=sys.stderr)
            sys.exit(1)

        # 4. Start dtntrigger to capture the outer/forwarded bundle at dtnforwardtarget
        print(f"[*] Spawning dtntrigger on {target_service_name} to capture forwarded payload...")
        trigger_proc = subprocess.Popen(
            [
                "cargo",
                "run",
                "--bin",
                "dtntrigger",
                "--",
                "-e",
                target_service_name,
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

        # 5. Send test bundle to dtnforwardtest
        test_payload = b"Hello BIBE!"
        print(f"[*] Sending test bundle to {forward_eid}...")
        send_proc = subprocess.run(
            [
                "cargo",
                "run",
                "--bin",
                "dtnsend",
                "--",
                "--receiver",
                forward_eid,
                "-v",
            ],
            input=test_payload,
            capture_output=True,
        )

        if send_proc.returncode != 0:
            print("[-] Error: dtnsend failed.", file=sys.stderr)
            print(send_proc.stderr.decode(), file=sys.stderr)
            sys.exit(1)

        print("[+] Test bundle successfully dispatched.")

        # 6. Poll for captured forwarded payload
        print("[*] Waiting for forwarded bundle to be routed and captured...")
        payload_received = False
        timeout = time.time() + 10.0
        while time.time() < timeout:
            if os.path.exists(captured_payload_path):
                payload_received = True
                break
            time.sleep(0.5)

        if not payload_received:
            print("[-] Error: Request timed out. No response captured.", file=sys.stderr)
            sys.exit(1)

        # 7. Verify the captured payload
        print("[*] Validating captured BIBE inner bundle...")
        with open(captured_payload_path, "rb") as f:
            inner_bundle_bytes = f.read()

        # The inner bundle bytes (which are wrapped in standard BIBE PDU) must contain our test_payload string
        assert test_payload in inner_bundle_bytes, "Inner bundle payload data mismatch!"
        print("[+] Found inner bundle payload matching the original text inside BIBE PDU.")

        # Decode as CBOR to ensure it's a valid BIBE PDU structure
        try:
            decoded_pdu = cbor2.loads(inner_bundle_bytes)
            if isinstance(decoded_pdu, cbor2.CBORTag):
                decoded_pdu = decoded_pdu.value
            print(f"[+] Decoded BIBE PDU: {decoded_pdu}")
            assert isinstance(decoded_pdu, list) and len(decoded_pdu) >= 2, "BIBE PDU is not a valid CBOR array structure"
            assert decoded_pdu[0] == 64443, f"Unexpected administrative record type: {decoded_pdu[0]}"
            pdu_content = decoded_pdu[1]
            assert isinstance(pdu_content, list) and len(pdu_content) >= 3, "Invalid BIBE PDU content array"
            encapsulated_bundle = pdu_content[2]
            assert isinstance(encapsulated_bundle, bytes), "Encapsulated bundle is not bytes"
            
            # Also deserialize the encapsulated inner bundle
            inner_decoded = cbor2.loads(encapsulated_bundle)
            if isinstance(inner_decoded, cbor2.CBORTag):
                inner_decoded = inner_decoded.value
            assert isinstance(inner_decoded, list), "Inner bundle is not a valid CBOR array"
            print("[+] Successfully parsed the encapsulated inner bundle CBOR.")
        except Exception as e:
            print(f"[-] Error: Failed to parse BIBE PDU or inner bundle: {e}", file=sys.stderr)
            sys.exit(1)

        print("[+] E2E Test completed successfully!")

    finally:
        # Tear down background services
        print("[*] Cleaning up background processes and temporary directories...")
        if forward_proc:
            try:
                forward_proc.terminate()
                forward_proc.wait()
            except Exception:
                pass

        if trigger_proc:
            try:
                trigger_proc.terminate()
                trigger_proc.wait()
            except Exception:
                pass

        # Remove any lingering .bin files in the workspace directory
        for f in ["inner.bin", "bibe_payload.bin"]:
            fpath = os.path.join(workspace_dir, f)
            if os.path.exists(fpath):
                try:
                    os.remove(fpath)
                    print(f"[+] Cleaned up lingering file: {f}")
                except Exception as e:
                    print(f"[-] Failed to clean up {f}: {e}", file=sys.stderr)

        shutil.rmtree(temp_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
