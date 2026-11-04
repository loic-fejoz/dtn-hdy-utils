#!/usr/bin/env bash
#
# test_bidirectional_direwolf.sh - Test de transfert bidirectionnel de bundles via LTP / Direwolf (9600 baud G3RUH)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

BIN_HAM="${ROOT_DIR}/target/debug/dtn-hdy-ham"
if [[ ! -x "${BIN_HAM}" ]]; then
    BIN_HAM="${ROOT_DIR}/target/release/dtn-hdy-ham"
fi

if [[ ! -x "${BIN_HAM}" ]]; then
    echo "[*] Compilation de dtn-hdy-ham..."
    cargo build -p dtn-hdy-ham
    BIN_HAM="${ROOT_DIR}/target/debug/dtn-hdy-ham"
fi

CONFIG_STATA="${SCRIPT_DIR}/stata-dw-ltp.yaml"
CONFIG_STATB="${SCRIPT_DIR}/statb-dw-ltp.yaml"

LOG_DIR="/tmp/dw_ltp_test_logs"
mkdir -p "${LOG_DIR}"

LOG_STATA="${LOG_DIR}/stata.log"
LOG_STATB="${LOG_DIR}/statb.log"

STARTED_DW=0
PID_DW_SCRIPT=0

# Vérifier si les deux instances Direwolf tournent déjà
if ! nc -z 127.0.0.1 8001 2>/dev/null || ! nc -z 127.0.0.1 8101 2>/dev/null; then
    echo "[*] Direwolf non détecté sur les ports 8001/8101. Lancement via launch_dual_direwolf.sh..."
    "${SCRIPT_DIR}/launch_dual_direwolf.sh" > "${LOG_DIR}/dual_direwolf.log" 2>&1 &
    PID_DW_SCRIPT=$!
    STARTED_DW=1
    
    # Attente que les ports KISS soient disponibles
    retries=60
    while ! nc -z 127.0.0.1 8001 2>/dev/null || ! nc -z 127.0.0.1 8101 2>/dev/null; do
        sleep 0.2
        retries=$((retries - 1))
        if [[ ${retries} -le 0 ]]; then
            echo "[-] Timeout en attendant le démarrage de Direwolf."
            exit 1
        fi
    done
    echo "[+] Direwolf opérationnel sur les ports 8001 et 8101."
fi

# Préparation des répertoires de stockage
rm -rf /tmp/dtn-stata /tmp/dtn-statb
mkdir -p /tmp/dtn-stata /tmp/dtn-statb

PID_STA=0
PID_STB=0

cleanup() {
    echo ""
    echo "[*] Nettoyage des processus..."
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

    if [[ ${STARTED_DW} -eq 1 && ${PID_DW_SCRIPT} -gt 0 ]]; then
        echo "[*] Arrêt du lanceur Direwolf..."
        kill -TERM "${PID_DW_SCRIPT}" 2>/dev/null || true
        wait "${PID_DW_SCRIPT}" 2>/dev/null || true
    fi
}

trap cleanup EXIT INT TERM

# Démarrage de Station A
echo "[*] Démarrage du BPA Station A (STATA)..."
"${BIN_HAM}" --config "${CONFIG_STATA}" > "${LOG_STATA}" 2>&1 &
PID_STA=$!

# Démarrage de Station B
echo "[*] Démarrage du BPA Station B (STATB)..."
"${BIN_HAM}" --config "${CONFIG_STATB}" > "${LOG_STATB}" 2>&1 &
PID_STB=$!

# Attente de la disponibilité des APIs Web
wait_web() {
    local port=$1
    local name=$2
    local retries=50
    while ! curl -s "http://127.0.0.1:${port}/api/v1/config" >/dev/null 2>&1; do
        sleep 0.1
        retries=$((retries - 1))
        if [[ ${retries} -le 0 ]]; then
            echo "[-] Timeout en attendant l'API Web de ${name} sur le port ${port}."
            exit 1
        fi
    done
}

wait_web 8081 "Station A"
wait_web 8082 "Station B"
echo "[+] Les deux instances Hardy Ham BPA sont prêtes."

# 1. Envoi STATA -> STATB
MSG_A_TO_B="Hello STATB from STATA via G3RUH Direwolf LTP!"
echo ""
echo "=================================================================="
echo " [TEST 1/2] Envoi de bundle : STATA -> STATB"
echo "=================================================================="
RESP_1=$(curl -s -X POST "http://127.0.0.1:8081/api/v1/bundles" \
  -H "Content-Type: application/json" \
  -d "{\"destination\": \"dtn://statb.ampr.org/chat\", \"payload\": \"${MSG_A_TO_B}\"}")
echo "[*] Réponse STATA : ${RESP_1}"

# Attente et vérification de la réception sur STATB
echo "[*] En attente de la réception sur STATB..."
RECV_B=0
for i in {1..40}; do
    if find /tmp/dtn-statb/payloads -type f 2>/dev/null | grep -q .; then
        if grep -rq "${MSG_A_TO_B}" /tmp/dtn-statb/payloads/ 2>/dev/null; then
            RECV_B=1
            break
        fi
    fi
    sleep 0.2
done

if [[ ${RECV_B} -eq 1 ]]; then
    echo "[+] SUCCÈS : STATB a bien reçu le bundle de STATA !"
else
    echo "[-] ÉCHEC : STATB n'a pas reçu le bundle dans le délai imparti."
    echo "=== LOG STATA ==="
    cat "${LOG_STATA}" | tail -n 20
    echo "=== LOG STATB ==="
    cat "${LOG_STATB}" | tail -n 20
    exit 1
fi

# 2. Envoi STATB -> STATA (Bidirectionnel)
MSG_B_TO_A="Hello STATA from STATB in return via G3RUH Direwolf LTP!"
echo ""
echo "=================================================================="
echo " [TEST 2/2] Envoi de bundle en retour : STATB -> STATA"
echo "=================================================================="
RESP_2=$(curl -s -X POST "http://127.0.0.1:8082/api/v1/bundles" \
  -H "Content-Type: application/json" \
  -d "{\"destination\": \"dtn://stata.ampr.org/chat\", \"payload\": \"${MSG_B_TO_A}\"}")
echo "[*] Réponse STATB : ${RESP_2}"

# Attente et vérification de la réception sur STATA
echo "[*] En attente de la réception sur STATA..."
RECV_A=0
for i in {1..40}; do
    if find /tmp/dtn-stata/payloads -type f 2>/dev/null | grep -q .; then
        if grep -rq "${MSG_B_TO_A}" /tmp/dtn-stata/payloads/ 2>/dev/null; then
            RECV_A=1
            break
        fi
    fi
    sleep 0.2
done

if [[ ${RECV_A} -eq 1 ]]; then
    echo "[+] SUCCÈS : STATA a bien reçu le bundle en retour de STATB !"
else
    echo "[-] ÉCHEC : STATA n'a pas reçu le bundle de retour dans le délai imparti."
    echo "=== LOG STATA ==="
    cat "${LOG_STATA}" | tail -n 20
    echo "=== LOG STATB ==="
    cat "${LOG_STATB}" | tail -n 20
    exit 1
fi

echo ""
echo "=================================================================="
echo "  RÉSULTAT DU TEST : 100% SUCCÈS TRANSFERT BIDIRECTIONNEL LTP / DIREWOLF"
echo "=================================================================="
echo "  STATA -> STATB : OK (\"${MSG_A_TO_B}\")"
echo "  STATB -> STATA : OK (\"${MSG_B_TO_A}\")"
echo "=================================================================="
