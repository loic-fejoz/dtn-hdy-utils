#!/bin/bash
# Desktop notification trigger helper for dtntrigger
# Argument 1: source EID
# Argument 2: path to temp file containing payload
SOURCE_EID="$1"
PAYLOAD_FILE="$2"
PAYLOAD_CONTENT=$(cat "$PAYLOAD_FILE")

notify-send "DTN Bundle from $SOURCE_EID" "$PAYLOAD_CONTENT"

# Run the command and capture output
response=$(agy -p "${PAYLOAD_CONTENT}")

# Send the response back to the sender
echo "$response" | ./dtnsend -s agy -r "${SOURCE_EID}"
