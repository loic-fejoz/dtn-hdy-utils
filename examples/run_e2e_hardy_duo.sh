#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${ROOT_DIR}/target/debug/dtn-hdy-ham"

KEEP_ALIVE=false
if [[ "${1:-}" == "--keep-alive" || "${1:-}" == "-k" ]]; then
    KEEP_ALIVE=true
fi

if [[ ! -x "${BIN}" ]]; then
    echo "Compilation du binaire dtn-hdy-ham..."
    cargo build -p dtn-hdy-ham --bin dtn-hdy-ham
fi

TMP_DIR="$(mktemp -d -t hardy-e2e-XXXXXX)"
STATA_CFG="${TMP_DIR}/stata.yaml"
STATB_CFG="${TMP_DIR}/statb.yaml"
STATA_LOG="${TMP_DIR}/stata.log"
STATB_LOG="${TMP_DIR}/statb.log"

echo "Configuration de l'environnement de test dans : ${TMP_DIR}"

cat << 'EOF' > "${STATB_CFG}"
callsign: "STATB"
node-id: "dtn://statb/"
storage:
  in-memory: true
grpc:
  enabled: true
  address: "127.0.0.1:25052"
web:
  address: "127.0.0.1:28082"
senml:
  metrics:
    - name: "urn:dtn:stata:power/bus_voltage"
      label: "Tension Bus A"
      unit: "V"
clas:
  - name: "tcpcl-statb"
    type: "tcpclv4"
    address: "127.0.0.1:24552"
    require-tls: false
routes:
  - pattern: "dtn://stata/*"
    cla: "tcpcl-statb"
    next-hop: "dtn://stata/"
rfc9171-validity:
  primary-block-integrity: false
  bundle-age-required: false
EOF

cat << 'EOF' > "${STATA_CFG}"
callsign: "STATA"
node-id: "dtn://stata/"
storage:
  in-memory: true
grpc:
  enabled: true
  address: "127.0.0.1:25051"
web:
  address: "127.0.0.1:28081"
clas:
  - name: "tcpcl-stata"
    type: "tcpclv4"
    address: "127.0.0.1:24551"
    require-tls: false
    peers:
      - "127.0.0.1:24552"
routes:
  - pattern: "dtn://statb/*"
    cla: "tcpcl-stata"
    next-hop: "dtn://statb/"
rfc9171-validity:
  primary-block-integrity: false
  bundle-age-required: false
EOF

PID_B=""
PID_A=""

cleanup() {
    if [ "$KEEP_ALIVE" = false ]; then
        echo -e "\nArrêt des instances STATA et STATB..."
        if [ -n "$PID_A" ] && kill -0 "$PID_A" 2>/dev/null; then
            kill "$PID_A" 2>/dev/null || true
        fi
        if [ -n "$PID_B" ] && kill -0 "$PID_B" 2>/dev/null; then
            kill "$PID_B" 2>/dev/null || true
        fi
        rm -rf "${TMP_DIR}"
        echo "Nettoyage terminé."
    else
        echo -e "\nMode --keep-alive actif: Les deux instances continuent de tourner."
        echo "STATA Web: http://127.0.0.1:28081 (PID: $PID_A, logs: $STATA_LOG)"
        echo "STATB Web: http://127.0.0.1:28082 (PID: $PID_B, logs: $STATB_LOG)"
        echo "Appuyez sur Ctrl+C pour arrêter les serveurs."
        wait
    fi
}
trap cleanup EXIT INT TERM

echo "1. Démarrage de STATB (port web: 28082, tcpcl: 24552)..."
"${BIN}" --config "${STATB_CFG}" > "${STATB_LOG}" 2>&1 &
PID_B=$!

echo "2. Démarrage de STATA (port web: 28081, tcpcl: 24551)..."
"${BIN}" --config "${STATA_CFG}" > "${STATA_LOG}" 2>&1 &
PID_A=$!

echo "3. Attente de la stabilisation et de la liaison TCPCLv4..."
sleep 2

echo "4. Exécution de la batterie de tests End-to-End..."
python3 "${SCRIPT_DIR}/test_e2e_hardy_duo.py"

echo -e "\nSuccès de la suite E2E !"
