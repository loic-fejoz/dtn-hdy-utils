#!/usr/bin/env python3
"""
Matrix notification trigger script for dtntrigger.
Receives the source EID and the temporary file path of the bundle payload,
and forwards it to a Matrix room using the matrix-nio library.
"""

import asyncio
import json
import logging
import os
import sys
from nio import AsyncClient, ErrorResponse

# Suppress verbose/validation warnings from matrix-nio library
logging.getLogger("nio").setLevel(logging.CRITICAL)

async def main():
    if len(sys.argv) < 3:
        print("Usage: dtn-matrix.py <source_eid> <temp_file_path>", file=sys.stderr)
        sys.exit(1)

    source_eid = sys.argv[1]
    temp_file_path = sys.argv[2]

    # Resolve config file path (prefer MATRIX_CONFIG environment variable,
    # falling back to matrix_config.json in current directory or script directory)
    config_path = os.environ.get("MATRIX_CONFIG", "matrix_config.json")
    if not os.path.exists(config_path):
        config_path = os.path.join(os.path.dirname(__file__), "matrix_config.json")

    if not os.path.exists(config_path):
        print(
            f"Error: Matrix config file not found. Create a matrix_config.json "
            f"file based on matrix_config.json.example.",
            file=sys.stderr,
        )
        sys.exit(1)

    try:
        with open(config_path, "r") as f:
            config = json.load(f)
    except Exception as e:
        print(f"Error reading configuration file: {e}", file=sys.stderr)
        sys.exit(1)

    # Allow overriding credentials via environment variables for better security
    if "MATRIX_ACCESS_TOKEN" in os.environ:
        config["access_token"] = os.environ["MATRIX_ACCESS_TOKEN"]
    if "MATRIX_PASSWORD" in os.environ:
        config["password"] = os.environ["MATRIX_PASSWORD"]

    # Read payload content
    try:
        with open(temp_file_path, "r", encoding="utf-8", errors="replace") as f:
            payload = f.read()
    except Exception as e:
        print(f"Error reading payload file: {e}", file=sys.stderr)
        sys.exit(1)

    message = f"DTN Bundle from {source_eid}\n\n{payload}"

    # Resolve store path and ensure the directory exists
    default_store = os.path.join(os.path.dirname(config_path), "matrix_store")
    store_path = config.get("store_path", default_store)
    if store_path and not os.path.exists(store_path):
        os.makedirs(store_path, exist_ok=True)

    # Initialize client and load the encryption store
    client = AsyncClient(
        config["homeserver"],
        config["user_id"],
        device_id=config.get("device_id", ""),
        store_path=store_path,
    )

    async def perform_password_login():
        if "password" not in config:
            print(
                "Error: Access token is invalid/expired, and no password was provided in config.\n"
                "Please either update 'access_token' or add 'password' to examples/matrix_config.json "
                "for automatic token renewal.",
                file=sys.stderr,
            )
            sys.exit(1)

        print("Logging in with password to obtain a fresh access token...", file=sys.stderr)
        # Clear device ID when performing a new login if not explicitly configured
        login_resp = await client.login(
            password=config["password"],
            device_name="dtntrigger",
        )
        if isinstance(login_resp, ErrorResponse):
            print(f"Error logging in with password: {login_resp}", file=sys.stderr)
            sys.exit(1)

        # Update client credentials and state
        client.access_token = login_resp.access_token
        client.device_id = login_resp.device_id
        client.user_id = login_resp.user_id
        client.load_store()

        # Save refreshed credentials to the config file
        config["access_token"] = login_resp.access_token
        config["device_id"] = login_resp.device_id
        try:
            with open(config_path, "w") as f:
                json.dump(config, f, indent=2)
            print("Successfully updated access token and device ID in config file.", file=sys.stderr)
        except Exception as e:
            print(f"Warning: Could not save updated config file: {e}", file=sys.stderr)

    # If access token is present, initialize client properties
    has_token = bool(config.get("access_token"))
    if has_token:
        client.access_token = config["access_token"]
        client.user_id = config["user_id"]
        client.load_store()
    else:
        await perform_password_login()

    try:
        # Sync to fetch room encryption state and keys
        sync_resp = await client.sync(timeout=3000)
        if isinstance(sync_resp, ErrorResponse):
            if sync_resp.status_code == "M_UNKNOWN_TOKEN" and "password" in config:
                await perform_password_login()
                sync_resp = await client.sync(timeout=3000)
                if isinstance(sync_resp, ErrorResponse):
                    print(f"Error syncing with homeserver after login: {sync_resp}", file=sys.stderr)
                    sys.exit(1)
            else:
                print(f"Error syncing with homeserver: {sync_resp}", file=sys.stderr)
                sys.exit(1)

        resp = await client.room_send(
            room_id=config["room_id"],
            message_type="m.room.message",
            content={"msgtype": "m.text", "body": message},
            ignore_unverified_devices=True,
        )

        if isinstance(resp, ErrorResponse):
            if resp.status_code == "M_UNKNOWN_TOKEN" and "password" in config:
                await perform_password_login()
                await client.sync(timeout=3000)
                resp = await client.room_send(
                    room_id=config["room_id"],
                    message_type="m.room.message",
                    content={"msgtype": "m.text", "body": message},
                    ignore_unverified_devices=True,
                )
                if isinstance(resp, ErrorResponse):
                    print(f"Error sending message to Matrix after login: {resp}", file=sys.stderr)
                    sys.exit(1)
            else:
                print(f"Error sending message to Matrix: {resp}", file=sys.stderr)
                sys.exit(1)

        print(f"Successfully routed notification to Matrix: {resp}")
    except Exception as e:
        print(f"Error sending message to Matrix: {e}", file=sys.stderr)
        sys.exit(1)
    finally:
        await client.close()

if __name__ == "__main__":
    asyncio.run(main())
