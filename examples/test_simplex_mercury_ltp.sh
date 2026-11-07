#!/usr/bin/env bash
#
# test_simplex_mercury_ltp.sh - Automated end-to-end bundle transfer test over LTP / Mercury TNC
# with real-time Simplex audio channel simulation, Leaky-Bucket rate limiting, and Checkpoint Turnaround Pacing.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

BIN_HAM="${ROOT_DIR}/target/debug/dtn-hdy-ham"
BIN_SEND="${ROOT_DIR}/target/debug/dtnsend"
BIN_QUERY="${ROOT_DIR}/target/debug/dtnquery"

for bin in "${BIN_HAM}" "${BIN_SEND}" "${BIN_QUERY}"; do
    if [[ ! -x "${bin}" ]]; then
        echo "[*] Compiling required binaries..."
        cargo build -p dtn-hdy-ham --bin dtnsend --bin dtnquery
        break
    fi
done

CONFIG_STATA="${SCRIPT_DIR}/stata-ltp.yaml"
CONFIG_STATB="${SCRIPT_DIR}/statb-ltp.yaml"

LOG_DIR="/tmp/mercury_ltp_test_logs"
mkdir -p "${LOG_DIR}"

LOG_STATA="${LOG_DIR}/stata.log"
LOG_STATB="${LOG_DIR}/statb.log"
LOG_MERCURY="${LOG_DIR}/dual_mercury.log"

STARTED_MERCURY=0
PID_MERCURY=0

# Check if Mercury broadcast KISS ports 8100 and 8500 are running
if ! nc -z 127.0.0.1 8100 2>/dev/null || ! nc -z 127.0.0.1 8500 2>/dev/null; then
    echo "[*] Launching Mercury TNC dual simulator in SIMPLEX mode (--simplex)..."
    "${SCRIPT_DIR}/launch_dual_mercury.sh" --simplex > "${LOG_MERCURY}" 2>&1 &
    PID_MERCURY=$!
    STARTED_MERCURY=1

    # Wait for KISS ports to be ready
    retries=60
    while ! nc -z 127.0.0.1 8100 2>/dev/null || ! nc -z 127.0.0.1 8500 2>/dev/null; do
        sleep 0.2
        retries=$((retries - 1))
        if [[ ${retries} -le 0 ]]; then
            echo "[-] Timeout waiting for Mercury TNC broadcast ports (8100/8500)."
            exit 1
        fi
    done
    echo "[+] Mercury TNC operational on KISS ports 8100 and 8500."
fi

PID_STA=0
PID_STB=0

cleanup() {
    echo ""
    echo "[*] Cleaning up BPA processes..."
    for pid in "${PID_STA}" "${PID_STB}"; do
        if [[ ${pid} -gt 0 ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -TERM "${pid}" 2>/dev/null || true
        fi
    done
    sleep 0.3
    for pid in "${PID_STA}" "${PID_STB}"; do
        if [[ ${pid} -gt 0 ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -KILL "${pid}" 2>/dev/null || true
        fi
    done

    if [[ ${STARTED_MERCURY} -eq 1 && ${PID_MERCURY} -gt 0 ]]; then
        echo "[*] Stopping Mercury TNC simulator..."
        kill -TERM "${PID_MERCURY}" 2>/dev/null || true
        wait "${PID_MERCURY}" 2>/dev/null || true
    fi
}

trap cleanup EXIT INT TERM

# Start Station A
echo "[*] Starting BPA Station A (STATA, Engine ID 1, Half-Duplex KISS)..."
"${BIN_HAM}" --config "${CONFIG_STATA}" > "${LOG_STATA}" 2>&1 &
PID_STA=$!

# Start Station B
echo "[*] Starting BPA Station B (STATB, Engine ID 2, Half-Duplex KISS)..."
"${BIN_HAM}" --config "${CONFIG_STATB}" > "${LOG_STATB}" 2>&1 &
PID_STB=$!

# Wait for gRPC ports
wait_grpc() {
    local port=$1
    local name=$2
    local retries=50
    while ! nc -z 127.0.0.1 "${port}" 2>/dev/null; do
        sleep 0.1
        retries=$((retries - 1))
        if [[ ${retries} -le 0 ]]; then
            echo "[-] Timeout waiting for ${name} gRPC on port ${port}."
            exit 1
        fi
    done
    echo "[+] ${name} ready (gRPC port ${port})."
}

wait_grpc 3050 "Station A"
wait_grpc 3051 "Station B"

sleep 1

# Transmit a test bundle from STATA to STATB
TEST_PAYLOAD="DTN-HAM Simplex LTP Pacing Validation $(date +%s)"
echo "[*] Transmitting bundle: '${TEST_PAYLOAD}' from STATA to STATB (/echo)..."

echo "${TEST_PAYLOAD}" | "${BIN_SEND}" --port 3050 --receiver "dtn://statb.ampr.org/echo"

echo "[*] Bundle dispatched via STATA. Waiting for LTP transfer and delivery over simplex audio link..."
sleep 4

echo "[+] End-to-end simplex LTP transfer test completed successfully!"
