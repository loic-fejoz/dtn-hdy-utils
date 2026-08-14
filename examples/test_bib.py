#!/usr/bin/env python3
import subprocess
import time
import os
import sys
import tempfile
import json
import cbor2
import signal

def query_local_eid():
    # Query node EID using dtnprint directly
    try:
        proc = subprocess.Popen(
            ["target/debug/dtnprint", "-s", "dummy-probe"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
        # Wait a moment for it to print registration EID, then kill it cleanly via SIGINT
        time.sleep(2)
        proc.send_signal(signal.SIGINT)
        stdout, stderr = proc.communicate()
        # Parse EID from stderr (e.g., "Listening for bundles on: dtn://N0CALL-2/dummy-probe")
        for line in stderr.splitlines():
            if "Listening for bundles on:" in line:
                parts = line.split("Listening for bundles on:")
                eid = parts[1].strip()
                # Extract node base EID
                if "/dummy-probe" in eid:
                    return eid.split("dummy-probe")[0]
        # Fallback if not found
        return "dtn://N0CALL-2/"
    except Exception as e:
        print(f"[-] Failed to auto-resolve EID: {e}, falling back to dtn://N0CALL-2/")
        return "dtn://N0CALL-2/"

def main():
    print("[*] Starting dtnbib end-to-end integration test...")
    
    # 1. Compile binaries first
    print("[*] Building binaries...")
    subprocess.run(["cargo", "build", "--bin", "dtnprint", "--bin", "dtnbib", "--bin", "dtnsend", "--bin", "dtntrigger"], check=True)

    local_node_eid = query_local_eid()
    print(f"[+] Detected local node EID: {local_node_eid}")
    
    print("[*] Waiting 5 seconds for Hardy to stabilize...")
    time.sleep(5.0)

    temp_dir = tempfile.mkdtemp(prefix="dtnbib_test_")
    print(f"[I] Temp directory: {temp_dir}")

    bib_proc = None
    trigger_proc = None
    try:

        # 2. Forge the inner bundle CBOR bytes
        # The inner bundle is destined for dtn://N0CALL/chat
        print("[*] Generating raw inner bundle CBOR using dtnsend --dry-run...")
        test_payload = "Hello from the BIBE tunnel!"
        
        # Run dtnsend in dry-run mode to output the CBOR hex of the inner bundle
        inner_dest_eid = f"{local_node_eid}dtnbibtestchat"
        dry_run_res = subprocess.run(
            ["target/debug/dtnsend", "-s", "dtnbibtestchat", "-r", inner_dest_eid, "--dryrun", "--sign-key", "my-secret-hamradio-key"],
            input=test_payload,
            capture_output=True,
            text=True,
            check=True
        )
        
        lines = dry_run_res.stdout.strip().splitlines()
        inner_bundle_hex = lines[1]
        inner_bundle_bytes = bytes.fromhex(inner_bundle_hex)
        print(f"[+] Generated inner bundle of {len(inner_bundle_bytes)} bytes.")

        # 3. Encapsulate the inner bundle in a standard BIBE PDU (Administrative Record 64443)
        # Format: [64443, [transmission-id, retransmission-time, encapsulated-bundle]]
        transmission_id = 12345
        retransmission_time = 0
        pdu_content = [transmission_id, retransmission_time, inner_bundle_bytes]
        bibe_pdu = [64443, pdu_content]
        
        bibe_pdu_bytes = cbor2.dumps(bibe_pdu)
        print(f"[+] Encapsulated into standard BIBE PDU ({len(bibe_pdu_bytes)} bytes).")

        # Write BIBE PDU payload to a file
        pdu_file_path = os.path.join(temp_dir, "bibe_pdu.bin")
        with open(pdu_file_path, "wb") as f:
            f.write(bibe_pdu_bytes)

        # 4. Spawn dtnbib receiver service
        # It listens on service 'bibe', and maps alias 'dtn://N0CALL/' to local EID
        print(f"[*] Spawning dtnbib: bibe -> chat with alias rewriting dtn://N0CALL/...")
        bib_proc = subprocess.Popen(
            ["target/debug/dtnbib", "--service", "bibe", "--alias", "dtn://N0CALL/", "--verify-key", "my-secret-hamradio-key", "-v"]
        )
        time.sleep(2.0) # Wait for registration

        # 5. Spawn dtntrigger on local service 'chat' to capture the re-injected inner bundle
        captured_payload_path = os.path.join(temp_dir, "captured_inner.bin")
        
        capture_script = os.path.join(temp_dir, "capture_chat.py")
        with open(capture_script, "w") as f:
            f.write(f"""import sys, shutil
# dtntrigger passes source EID as arg 1, and filepath containing payload as arg 2
source_eid = sys.argv[1]
temp_file = sys.argv[2]
shutil.copy(temp_file, "{captured_payload_path}")
print("[trigger] Captured inner payload!")
""")

        print(f"[*] Spawning dtntrigger on local dtnbibtestchat service EID...")
        trigger_proc = subprocess.Popen(
            ["target/debug/dtntrigger", "-e", "dtnbibtestchat", "-c", f"python3 {capture_script}", "--verify-key", "my-secret-hamradio-key", "-v"]
        )
        time.sleep(2.0) # Wait for registration

        # 6. Send the BIBE PDU file to the 'bibe' service EID
        bibe_service_eid = f"{local_node_eid}bibe"
        print(f"[*] Sending BIBE PDU to {bibe_service_eid}...")
        subprocess.run(
            ["target/debug/dtnsend", "-s", "dtnbibtestchat", "-r", bibe_service_eid, pdu_file_path],
            check=True
        )
        print("[+] Outer BIBE bundle sent.")

        # 7. Wait and verify
        print("[*] Waiting for dtnbib to decapsulate, rewrite and re-inject...")
        max_wait = 10
        payload_received = False
        for _ in range(max_wait):
            if os.path.exists(captured_payload_path):
                payload_received = True
                break
            time.sleep(1)

        if not payload_received:
            print("[-] Error: Request timed out. Inner bundle was not captured.", file=sys.stderr)
            sys.exit(1)

        # Verify the captured inner bundle payload
        print("[*] Validating captured inner bundle...")
        with open(captured_payload_path, "r", encoding="utf-8", errors="ignore") as f:
            content = f.read()
        
        assert test_payload in content, f"Payload mismatch! Expected '{test_payload}', got: '{content}'"
        print(f"[+] Successfully verified inner bundle payload: '{content}'")
        print("[+] E2E dtnbib Test completed successfully!")

    finally:
        # Cleanup processes
        print("[*] Cleaning up background processes and files...")
        if bib_proc:
            bib_proc.terminate()
            bib_proc.wait()
        if trigger_proc:
            trigger_proc.terminate()
            trigger_proc.wait()
        
        # Cleanup files
        for f in ["bibe_pdu.bin", "capture_chat.py", "captured_inner.bin"]:
            p = os.path.join(temp_dir, f)
            if os.path.exists(p):
                os.remove(p)
        try:
            os.rmdir(temp_dir)
        except Exception:
            pass

if __name__ == "__main__":
    main()
