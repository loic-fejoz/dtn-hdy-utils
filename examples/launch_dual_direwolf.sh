#!/usr/bin/env bash
#
# launch_dual_direwolf.sh - Lance 2 instances Direwolf reliées par audio croisée (PulseAudio/PipeWire, 9600 baud G3RUH)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONF_DIR="${SCRIPT_DIR}/direwolf"
LOG_DIR="${CONF_DIR}/logs"

CONFIG_A="${CONF_DIR}/dwa.conf"
CONFIG_B="${CONF_DIR}/dwb.conf"
ASOUND_CONF="${CONF_DIR}/asound.conf"

LOG_A="${LOG_DIR}/station_a.log"
LOG_B="${LOG_DIR}/station_b.log"

PORT_A_KISS=8001
PORT_B_KISS=8101

# Vérification du binaire direwolf
if ! command -v direwolf >/dev/null 2>&1; then
    echo "[-] Erreur : Le binaire 'direwolf' est introuvable dans le PATH."
    echo "    Veuillez installer Direwolf (e.g. apt-get install direwolf)."
    exit 1
fi

PID_PW=0
PID_WP=0
PID_PWP=0
PW_RUNTIME_DIR=""

# Détection et configuration du serveur PulseAudio / PipeWire
if [[ -z "${PULSE_SERVER:-}" ]]; then
    PULSE_SOCK="/run/user/$(id -u)/pulse/native"
    if [[ -S "${PULSE_SOCK}" ]]; then
        export PULSE_SERVER="unix:${PULSE_SOCK}"
    fi
fi

if ! command -v pactl >/dev/null 2>&1; then
    echo "[-] Erreur : L'utilitaire 'pactl' est introuvable."
    exit 1
fi

# Si le serveur PulseAudio / PipeWire n'est pas joignable, démarrage d'un serveur local temporaire
if ! pactl info >/dev/null 2>&1; then
    echo "[*] Aucun serveur PulseAudio/PipeWire actif détecté. Démarrage d'un serveur temporaire PipeWire..."
    PW_RUNTIME_DIR="/tmp/pw-direwolf-$$-$(id -u)"
    mkdir -p "${PW_RUNTIME_DIR}"
    export XDG_RUNTIME_DIR="${PW_RUNTIME_DIR}"
    export PULSE_COOKIE="${PW_RUNTIME_DIR}/pulse-cookie"

    if command -v pipewire >/dev/null 2>&1; then
        pipewire >/dev/null 2>&1 &
        PID_PW=$!
    fi
    if command -v wireplumber >/dev/null 2>&1; then
        wireplumber >/dev/null 2>&1 &
        PID_WP=$!
    fi
    if command -v pipewire-pulse >/dev/null 2>&1; then
        pipewire-pulse >/dev/null 2>&1 &
        PID_PWP=$!
    fi

    export PULSE_SERVER="unix:${PW_RUNTIME_DIR}/pulse/native"

    # Attente que pactl réponde
    pw_ready=0
    for i in {1..30}; do
        if pactl info >/dev/null 2>&1; then
            pw_ready=1
            break
        fi
        sleep 0.1
    done

    if [[ ${pw_ready} -eq 0 ]]; then
        echo "[-] Erreur : Impossible d'initialiser le serveur PipeWire audio temporaire."
        exit 1
    fi
    echo "[+] Serveur PipeWire temporaire opérationnel."
fi

mkdir -p "${LOG_DIR}"

# Nettoyage préalable des anciens modules dw_
echo "[*] Nettoyage des modules audio existants..."
for mod in $(pactl list short modules 2>/dev/null | grep "sink_name=dw_" | awk '{print $1}'); do
    pactl unload-module "$mod" 2>/dev/null || true
done

# Libération préalable des ports si des instances précédentes tournaient encore
fuser -k ${PORT_A_KISS}/tcp ${PORT_B_KISS}/tcp 2>/dev/null || true

# Chargement des null-sinks PulseAudio
echo "[*] Création des null-sinks audio croisés (dw_a_tx, dw_b_tx à 48000 Hz)..."
MOD_A=$(pactl load-module module-null-sink sink_name=dw_a_tx sink_properties=device.description=dw_a_tx rate=48000)
MOD_B=$(pactl load-module module-null-sink sink_name=dw_b_tx sink_properties=device.description=dw_b_tx rate=48000)

export ALSA_CONFIG_PATH="${ASOUND_CONF}"

PID_A=0
PID_B=0

cleanup() {
    echo ""
    echo "[*] Arrêt des instances Direwolf et libération des flux audio..."
    for pid in "${PID_A}" "${PID_B}"; do
        if [[ ${pid} -gt 0 ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -TERM "${pid}" 2>/dev/null || true
        fi
    done
    sleep 0.3
    for pid in "${PID_A}" "${PID_B}"; do
        if [[ ${pid} -gt 0 ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -KILL "${pid}" 2>/dev/null || true
        fi
    done

    if [[ -n "${MOD_A:-}" ]]; then
        pactl unload-module "${MOD_A}" 2>/dev/null || true
    fi
    if [[ -n "${MOD_B:-}" ]]; then
        pactl unload-module "${MOD_B}" 2>/dev/null || true
    fi

    # Arrêt du serveur PipeWire temporaire si lancé
    for pid in "${PID_PWP}" "${PID_WP}" "${PID_PW}"; do
        if [[ ${pid} -gt 0 ]] && kill -0 "${pid}" 2>/dev/null; then
            kill -TERM "${pid}" 2>/dev/null || true
        fi
    done
    if [[ -n "${PW_RUNTIME_DIR}" && -d "${PW_RUNTIME_DIR}" ]]; then
        rm -rf "${PW_RUNTIME_DIR}"
    fi

    echo "[+] Environnement Direwolf arrêté proprement."
}

trap cleanup EXIT INT TERM

echo "[*] Démarrage de Direwolf Station A (STATA)..."
direwolf -c "${CONFIG_A}" -r 48000 -t 0 > "${LOG_A}" 2>&1 &
PID_A=$!

echo "[*] Démarrage de Direwolf Station B (STATB)..."
direwolf -c "${CONFIG_B}" -r 48000 -t 0 > "${LOG_B}" 2>&1 &
PID_B=$!

# Attente que les ports KISS TCP soient prêts
wait_port() {
    local port=$1
    local name=$2
    local retries=50
    while ! nc -z 127.0.0.1 "${port}" 2>/dev/null; do
        sleep 0.1
        retries=$((retries - 1))
        if [[ ${retries} -le 0 ]]; then
            echo "[-] Timeout en attendant l'ouverture du port ${port} (${name})."
            exit 1
        fi
    done
}

wait_port "${PORT_A_KISS}" "Station A (KISS TCP)"
wait_port "${PORT_B_KISS}" "Station B (KISS TCP)"

cat << EOF
======================================================================
  DIREWOLF DUAL-INSTANCE SIMULATOR (AUDIO CROISÉE G3RUH 9600 BAUD)
======================================================================
  Instance A [STATA] (PID: ${PID_A}):
    - Port KISS TCP      : ${PORT_A_KISS}
    - Config             : ${CONFIG_A}
    - Log                : ${LOG_A}

  Instance B [STATB] (PID: ${PID_B}):
    - Port KISS TCP      : ${PORT_B_KISS}
    - Config             : ${CONFIG_B}
    - Log                : ${LOG_B}

  Pont Audio :
    - PipeWire/PulseAudio null-sinks croisés (dw_a_tx <-> dw_b_tx, 48000 Hz)
    - Modulation : FSK 9600 bauds (G3RUH)
    - Mode : KISS TCP brut (pas d'AX.25 ni d'FX.25)
======================================================================
Pour tester automatiquement un transfert de bundles :
  ./examples/test_bidirectional_direwolf.sh

Appuyez sur Ctrl+C pour arrêter les deux instances et quitter.
======================================================================
EOF

# Maintient le script actif tant que les deux instances tournent
while kill -0 "${PID_A}" 2>/dev/null && kill -0 "${PID_B}" 2>/dev/null; do
    sleep 1
done
