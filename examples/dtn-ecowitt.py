#!/usr/bin/env python3
"""
Ecowitt weather station data collector for DTN.
Fetches real-time sensor data from Ecowitt API, converts outdoor/indoor temperature
(from °F to °C) and outdoor humidity into a SenML (RFC 8428) CBOR payload, and sends
it to a DTN endpoint using dtnsend.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import urllib.parse
import urllib.request

import cbor2

def encode_cbor(data):
    return cbor2.dumps(data)

# SenML CBOR Label Keys (RFC 8428 Section 6)
SENML_KEY_BVER = -1
SENML_KEY_BN = -2
SENML_KEY_BT = -3
SENML_KEY_BU = -4
SENML_KEY_BV = -5
SENML_KEY_BS = -6
SENML_KEY_N = 0
SENML_KEY_U = 1
SENML_KEY_V = 2
SENML_KEY_VS = 3
SENML_KEY_VB = 4
SENML_KEY_S = 5
SENML_KEY_T = 6
SENML_KEY_UT = 7
SENML_KEY_VD = 8


def fahrenheit_to_celsius(f_temp: float) -> float:
    """Converts Fahrenheit to Celsius, rounded to 2 decimal places."""
    return round((f_temp - 32.0) * 5.0 / 9.0, 2)


def fetch_ecowitt_data(app_key: str, api_key: str, mac: str) -> dict:
    """Fetches real-time weather data from the Ecowitt API."""
    base_url = "https://api.ecowitt.net/api/v3/device/real_time"
    params = {
        "application_key": app_key,
        "api_key": api_key,
        "mac": mac,
        "call_back": "all",
    }
    url = f"{base_url}?{urllib.parse.urlencode(params)}"
    req = urllib.request.Request(url, headers={"User-Agent": "dtn-ecowitt/1.0"})
    with urllib.request.urlopen(req, timeout=15) as resp:
        body = resp.read().decode("utf-8")
        return json.loads(body)


def build_senml_pack(data_json: dict, base_urn: str) -> tuple[list[dict], list[dict]]:
    """
    Parses Ecowitt response and constructs SenML CBOR pack (with integer keys)
    and an equivalent JSON-friendly representation (with string keys).
    """
    data = data_json.get("data", {})
    outdoor = data.get("outdoor", {})
    indoor = data.get("indoor", {})

    # Base time: prefer outdoor temperature reading timestamp or top-level time
    base_time_str = (
        outdoor.get("temperature", {}).get("time")
        or indoor.get("temperature", {}).get("time")
        or data_json.get("time")
    )
    base_time = int(base_time_str) if base_time_str else None

    # Parse raw values
    out_temp_f = float(outdoor.get("temperature", {}).get("value", 0.0))
    in_temp_f = float(indoor.get("temperature", {}).get("value", 0.0))
    out_hum = float(outdoor.get("humidity", {}).get("value", 0.0))

    # Convert °F to °C
    out_temp_c = fahrenheit_to_celsius(out_temp_f)
    in_temp_c = fahrenheit_to_celsius(in_temp_f)

    # Build SenML Records using RFC 8428 integer labels for CBOR
    cbor_pack = []
    first_record = {
        SENML_KEY_BN: base_urn,
        SENML_KEY_N: "outdoor/temperature",
        SENML_KEY_U: "Cel",
        SENML_KEY_V: out_temp_c,
    }
    if base_time is not None:
        first_record[SENML_KEY_BT] = base_time
    cbor_pack.append(first_record)

    cbor_pack.append({
        SENML_KEY_N: "indoor/temperature",
        SENML_KEY_U: "Cel",
        SENML_KEY_V: in_temp_c,
    })

    cbor_pack.append({
        SENML_KEY_N: "outdoor/humidity",
        SENML_KEY_U: "/%",
        SENML_KEY_V: out_hum,
    })

    # Build equivalent human-readable JSON representation
    json_pack = []
    json_first = {
        "bn": base_urn,
        "n": "outdoor/temperature",
        "u": "Cel",
        "v": out_temp_c,
    }
    if base_time is not None:
        json_first["bt"] = base_time
    json_pack.append(json_first)

    json_pack.append({
        "n": "indoor/temperature",
        "u": "Cel",
        "v": in_temp_c,
    })

    json_pack.append({
        "n": "outdoor/humidity",
        "u": "/%",
        "v": out_hum,
    })

    return cbor_pack, json_pack


def find_dtnsend_cmd(port: int | None = None) -> list[str]:
    """Locates the dtnsend executable or falls back to cargo run."""
    script_dir = os.path.dirname(os.path.abspath(__file__))
    repo_root = os.path.abspath(os.path.join(script_dir, ".."))

    candidate_paths = [
        shutil.which("dtnsend"),
        os.path.join(repo_root, "target", "release", "dtnsend"),
        os.path.join(repo_root, "target", "debug", "dtnsend"),
        os.path.join(script_dir, "dtnsend"),
    ]

    cmd = None
    for candidate in candidate_paths:
        if candidate and os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            cmd = [candidate]
            break

    if cmd is None:
        cmd = ["cargo", "run", "--manifest-path", os.path.join(repo_root, "Cargo.toml"), "--bin", "dtnsend", "--"]

    if port is not None:
        cmd.extend(["-p", str(port)])

    return cmd


def main():
    parser = argparse.ArgumentParser(
        description="Fetch Ecowitt weather data, encode into SenML CBOR, and send to DTN endpoint."
    )
    parser.add_argument(
        "--app-key",
        default=os.environ.get("ECOWITT_APP_KEY", "app_key"),
        help="Ecowitt application key",
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("ECOWITT_API_KEY", "api_key"),
        help="Ecowitt API key",
    )
    parser.add_argument(
        "--mac",
        default=os.environ.get("ECOWITT_MAC", "AA:BB:CC:DD:EE:FF"),
        help="Ecowitt station MAC address",
    )
    parser.add_argument(
        "--receiver",
        "-r",
        default="dtn://N0CALL/senml",
        help="Receiver DTN EID (default: dtn://N0CALL/senml)",
    )
    parser.add_argument(
        "--base-urn",
        help="SenML base name URN (default: derived from station MAC)",
    )
    parser.add_argument(
        "--lifetime",
        "-l",
        "--ttl",
        type=int,
        default=3600,
        help="Bundle lifetime / time-to-live in seconds (default: 3600)",
    )
    parser.add_argument(
        "--input-file",
        "-i",
        help="Path to JSON file with API response (or '-' for stdin) instead of calling HTTP API",
    )
    parser.add_argument(
        "--port",
        "-p",
        type=int,
        help="gRPC port for Hardy BPA (default: 50051 or resolved from environment)",
    )
    parser.add_argument(
        "--dry-run",
        "-D",
        action="store_true",
        help="Print SenML JSON and CBOR hex payload without sending via dtnsend",
    )
    parser.add_argument(
        "--verbose",
        "-v",
        action="store_true",
        help="Verbose output",
    )

    args = parser.parse_args()

    # Retrieve Ecowitt response
    if args.input_file:
        if args.input_file == "-":
            raw_content = sys.stdin.read()
        else:
            with open(args.input_file, "r", encoding="utf-8") as f:
                raw_content = f.read()
        data_json = json.loads(raw_content)
    else:
        if args.verbose:
            print(f"Fetching data from Ecowitt API for MAC {args.mac}...", file=sys.stderr)
        data_json = fetch_ecowitt_data(args.app_key, args.api_key, args.mac)

    if data_json.get("code") != 0 and data_json.get("msg") != "success":
        print(f"Error from Ecowitt API: {data_json}", file=sys.stderr)
        sys.exit(1)

    base_urn = args.base_urn or f"urn:ecowitt:{args.mac.lower().replace(':', '-')}:"
    cbor_pack, json_pack = build_senml_pack(data_json, base_urn)
    cbor_bytes = encode_cbor(cbor_pack)

    if args.verbose or args.dry_run:
        print("SenML JSON representation:", file=sys.stderr)
        print(json.dumps(json_pack, indent=2), file=sys.stderr)
        print(f"SenML CBOR payload ({len(cbor_bytes)} bytes, hex):", file=sys.stderr)
        print(cbor_bytes.hex(), file=sys.stderr)

    if args.dry_run:
        return

    # Forward to dtnsend
    cmd = find_dtnsend_cmd(port=args.port)
    cmd.extend(["-r", args.receiver])
    if args.lifetime is not None:
        cmd.extend(["-l", str(args.lifetime)])
    if args.verbose:
        cmd.append("-v")
        print(f"Executing: {' '.join(cmd)}", file=sys.stderr)

    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    stdout, stderr = proc.communicate(input=cbor_bytes)

    if stdout:
        sys.stdout.write(stdout.decode("utf-8", errors="replace"))
    if stderr:
        sys.stderr.write(stderr.decode("utf-8", errors="replace"))

    if proc.returncode != 0:
        print(f"dtnsend failed with return code {proc.returncode}", file=sys.stderr)
        sys.exit(proc.returncode)

    if args.verbose:
        print(f"Successfully sent {len(cbor_bytes)} bytes SenML CBOR bundle to {args.receiver}", file=sys.stderr)


if __name__ == "__main__":
    main()
