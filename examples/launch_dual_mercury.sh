#!/usr/bin/env bash
#
# launch_dual_mercury.sh - Lance 2 instances de Mercury reliées par un pont audio temps-réel (8 kHz)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Détection de l'emplacement du projet Mercury
if [[ -n "${MERCURY_DIR:-}" && -d "${MERCURY_DIR}" ]]; then
    MERCURY_DIR="$(cd "${MERCURY_DIR}" && pwd)"
elif [[ -x "${SCRIPT_DIR}/mercury" ]]; then
    MERCURY_DIR="${SCRIPT_DIR}"
elif [[ -d "${SCRIPT_DIR}/../../mercury" ]]; then
    MERCURY_DIR="$(cd "${SCRIPT_DIR}/../../mercury" && pwd)"
elif [[ -d "${SCRIPT_DIR}/../mercury" ]]; then
    MERCURY_DIR="$(cd "${SCRIPT_DIR}/../mercury" && pwd)"
else
    echo "[-] Erreur : Répertoire du projet 'mercury' introuvable."
    echo "    Veuillez définir la variable d'environnement MERCURY_DIR."
    exit 1
fi

BIN_MERCURY="${MERCURY_DIR}/mercury"
SIM_DIR="${MERCURY_DIR}/simulation"
PIPE_DIR="${SIM_DIR}/pipes"
LOG_DIR="${SIM_DIR}/logs"

CONFIG_A="${SIM_DIR}/mercury_station_a.ini"
CONFIG_B="${SIM_DIR}/mercury_station_b.ini"

A_TX="${PIPE_DIR}/a_tx.fifo"
A_RX="${PIPE_DIR}/a_rx.fifo"
B_TX="${PIPE_DIR}/b_tx.fifo"
B_RX="${PIPE_DIR}/b_rx.fifo"

LOG_A="${LOG_DIR}/station_a.log"
LOG_B="${LOG_DIR}/station_b.log"
LOG_BRIDGE="${LOG_DIR}/bridge.log"

PORT_A_CTL=8300
PORT_A_DATA=8301
PORT_A_BCAST=8100

PORT_B_CTL=8400
PORT_B_DATA=8401
PORT_B_BCAST=8500

# Vérification du binaire
if [[ ! -x "${BIN_MERCURY}" ]]; then
    echo "[-] Erreur : Le binaire 'mercury' est introuvable ou non exécutable (${BIN_MERCURY})."
    echo "    Veuillez compiler le projet avec 'make' dans ${MERCURY_DIR} avant de lancer ce script."
    exit 1
fi

# Libération préalable des ports si des instances précédentes tournaient encore
fuser -k ${PORT_A_CTL}/tcp ${PORT_A_DATA}/tcp ${PORT_B_CTL}/tcp ${PORT_B_DATA}/tcp 2>/dev/null || true

# Création des répertoires nécessaires
mkdir -p "${PIPE_DIR}" "${LOG_DIR}"

# Préparation des tubes nommés (FIFOs)
echo "[*] Initialisation des tubes audio (FIFOs)..."
rm -f "${A_TX}" "${A_RX}" "${B_TX}" "${B_RX}"
mkfifo "${A_TX}" "${A_RX}" "${B_TX}" "${B_RX}"

PID_A=0
PID_B=0
PID_BRIDGE=0

cleanup() {
    echo ""
    echo "[*] Arrêt des instances Mercury et du pont audio..."
    for pid in "${PID_A}" "${PID_B}" "${PID_BRIDGE}"; do
        if [[ ${pid} -gt 0 ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -TERM "${pid}" 2>/dev/null || true
        fi
    done
    
    sleep 0.4
    for pid in "${PID_A}" "${PID_B}" "${PID_BRIDGE}"; do
        if [[ ${pid} -gt 0 ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -KILL "${pid}" 2>/dev/null || true
        fi
    done

    # Nettoyage des FIFOs
    rm -f "${A_TX}" "${A_RX}" "${B_TX}" "${B_RX}"
    echo "[+] Simulation arrêtée proprement."
}

trap cleanup EXIT INT TERM

# Démarrage des deux instances Mercury
echo "[*] Démarrage de la Station A (STATA)..."
"${BIN_MERCURY}" -C "${CONFIG_A}" > "${LOG_A}" 2>&1 &
PID_A=$!

echo "[*] Démarrage de la Station B (STATB)..."
"${BIN_MERCURY}" -C "${CONFIG_B}" > "${LOG_B}" 2>&1 &
PID_B=$!

# Démarrage du pont audio temps-réel (cadencé à 8000 Hz)
echo "[*] Démarrage du pont audio temps-réel..."
python3 "${SIM_DIR}/audio_bridge.py" "${A_TX}" "${B_RX}" "${B_TX}" "${A_RX}" > "${LOG_BRIDGE}" 2>&1 &
PID_BRIDGE=$!

# Attente que les ports de contrôle soient prêts
wait_port() {
    local port=$1
    local name=$2
    local retries=40
    while ! nc -z 127.0.0.1 "${port}" 2>/dev/null; do
        sleep 0.1
        retries=$((retries - 1))
        if [[ ${retries} -le 0 ]]; then
            echo "[-] Timeout en attendant l'ouverture du port ${port} (${name})."
            exit 1
        fi
    done
}

wait_port "${PORT_A_CTL}" "Station A (Contrôle)"
wait_port "${PORT_B_CTL}" "Station B (Contrôle)"

cat << EOF
======================================================================
  MERCURY DUAL-INSTANCE SIMULATOR (PONT AUDIO TEMPS-REEL 8 kHz)
======================================================================
  Instance A [STATA] (PID: ${PID_A}):
    - Port Contrôle TNC  : ${PORT_A_CTL}
    - Port Données ARQ   : ${PORT_A_DATA}
    - Port Broadcast     : ${PORT_A_BCAST}
    - Log                : ${LOG_A}

  Instance B [STATB] (PID: ${PID_B}):
    - Port Contrôle TNC  : ${PORT_B_CTL}
    - Port Données ARQ   : ${PORT_B_DATA}
    - Port Broadcast     : ${PORT_B_BCAST}
    - Log                : ${LOG_B}

  Pont Audio (PID: ${PID_BRIDGE}):
    - Flux cadencé à 8000 Hz (audio TX utile + silence RX continu)
======================================================================
Pour tester automatiquement le transfert d'un fichier :
  python3 test_arq_transfer.py [chemin_fichier_optionnel]

Appuyez sur Ctrl+C pour arrêter les deux instances et quitter.
======================================================================
EOF

# Maintient le script actif tant que les deux instances tournent
while kill -0 "${PID_A}" 2>/dev/null && kill -0 "${PID_B}" 2>/dev/null; do
    sleep 1
done
