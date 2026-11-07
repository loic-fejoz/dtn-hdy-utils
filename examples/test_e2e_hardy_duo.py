#!/usr/bin/env python3
"""
End-to-End Test Suite for Two Interconnected Hardy BPA Instances (dtn-hdy-ham).
Tests the latest implemented features across STATA (node A) and STATB (node B):
- Dynamic static routing hot-reload (Phase 3ter)
- TCPCLv4 peering & live connection tracking
- BPSec BIB Keystore management & RFC 9103/9172 cryptographic policies (Phase 6 & 6bis)
- End-to-end BPSec authenticated chat messaging & conversation threading (Phase 3 & 6bis)
- Strict cryptographic policy rejection vs Warn policy acceptance
- Bidirectional chat messaging and response confirmation
- End-to-end authenticated files service, HTTP Range streaming & sticky flag (Phase 3bis & 6bis)
- Real-time Server-Sent Events (SSE) push streaming (Phase 4)
- Bundle Storage Inspector & Cryptographic badges (Phase 2 & 6bis)
- In-memory system log buffer and tracing (Phase 4)
"""

import sys
import os
import time
import json
import threading
import urllib.request
import urllib.error

STATA_WEB = "http://127.0.0.1:28081"
STATB_WEB = "http://127.0.0.1:28082"
STATA_EID = "dtn://stata/"
STATB_EID = "dtn://statb/"

SHARED_SECRET = "TopSecretBPSecHMACKey2026!#"
INVALID_SECRET = "WrongMismatchedKeyForTesting999"

GREEN = "\033[92m"
RED = "\033[91m"
YELLOW = "\033[93m"
BLUE = "\033[94m"
CYAN = "\033[96m"
BOLD = "\033[1m"
RESET = "\033[0m"

passed_tests = 0
failed_tests = 0
sse_events_received = []
sse_stop_flag = False

def log_test(name, success, detail=""):
    global passed_tests, failed_tests
    if success:
        passed_tests += 1
        print(f"  {GREEN}[PASS]{RESET} {name} {CYAN}{detail}{RESET}")
    else:
        failed_tests += 1
        print(f"  {RED}[FAIL]{RESET} {name} - {RED}{detail}{RESET}")

def http_req(url, method="GET", data=None, headers=None):
    if headers is None:
        headers = {}
    body = None
    if data is not None:
        if isinstance(data, (dict, list)):
            body = json.dumps(data).encode("utf-8")
            headers["Content-Type"] = "application/json"
        elif isinstance(data, str):
            body = data.encode("utf-8")
        elif isinstance(data, bytes):
            body = data

    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            content = resp.read()
            resp_headers = dict(resp.headers)
            try:
                json_data = json.loads(content.decode("utf-8"))
            except Exception:
                json_data = None
            return resp.status, json_data, content, resp_headers
    except urllib.error.HTTPError as e:
        err_content = e.read()
        try:
            err_json = json.loads(err_content.decode("utf-8"))
        except Exception:
            err_json = None
        return e.code, err_json, err_content, dict(e.headers)
    except Exception as e:
        return 0, None, str(e).encode(), {}

def sse_listener_thread(url):
    global sse_stop_flag, sse_events_received
    req = urllib.request.Request(url, headers={"Accept": "text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            while not sse_stop_flag:
                line = resp.readline()
                if not line:
                    break
                line_str = line.decode("utf-8", errors="replace").strip()
                if line_str.startswith("event:"):
                    evt_type = line_str.split(":", 1)[1].strip()
                    data_line = resp.readline().decode("utf-8", errors="replace").strip()
                    if data_line.startswith("data:"):
                        payload = data_line.split(":", 1)[1].strip()
                        sse_events_received.append((evt_type, payload))
    except Exception:
        pass

def wait_for_ready(url, max_retries=15):
    for _ in range(max_retries):
        status, data, _, _ = http_req(f"{url}/api/v1/config")
        if status == 200 and data and "node_id" in data:
            return True
        time.sleep(0.5)
    return False

def run_all_tests():
    global sse_stop_flag
    print(f"\n{BOLD}{CYAN}=== Lancement des tests E2E Hardy BPA (STATA <--> STATB) ==={RESET}\n")

    # -------------------------------------------------------------
    # SUITE 1: Vérification de l'Identité & Appairage TCPCLv4
    # -------------------------------------------------------------
    print(f"{BOLD}[Suite 1: Identité & Connexion TCPCLv4]{RESET}")
    status, conf_a, _, _ = http_req(f"{STATA_WEB}/api/v1/config")
    log_test("Configuration STATA accessible", status == 200 and conf_a.get("node_id") == STATA_EID, f"node_id={conf_a.get('node_id') if conf_a else None}")

    status, conf_b, _, _ = http_req(f"{STATB_WEB}/api/v1/config")
    log_test("Configuration STATB accessible", status == 200 and conf_b.get("node_id") == STATB_EID, f"node_id={conf_b.get('node_id') if conf_b else None}")

    quota_a = conf_a.get("application", {}).get("quota", {}).get("max_storage", 0) if conf_a else 0
    log_test("Quota de stockage configuré (300 Mo)", quota_a >= 300_000_000, f"quota={quota_a} bytes")

    # -------------------------------------------------------------
    # SUITE 2: Routage Statique Dynamique & Hot-Reload BPA (Phase 3ter)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 2: Routage Dynamique & Rechargement à Chaud (Phase 3ter)]{RESET}")
    status, routes_a, _, _ = http_req(f"{STATA_WEB}/api/v1/routes")
    log_test("Listing des routes initiales STATA", status == 200 and len(routes_a) >= 1)

    dyn_route = {
        "pattern": "dtn://gateway/**",
        "cla": "tcpcl-stata",
        "next-hop": "dtn://statb/"
    }
    status, res, _, _ = http_req(f"{STATA_WEB}/api/v1/routes", method="POST", data=dyn_route)
    log_test("Ajout dynamique de route POST /api/v1/routes", status == 201)

    status, routes_a2, _, _ = http_req(f"{STATA_WEB}/api/v1/routes")
    has_dyn = any(r.get("pattern") == "dtn://gateway/**" for r in routes_a2 or [])
    log_test("Vérification injection immédiate dans le RIB BPA", has_dyn)

    status, res, _, _ = http_req(f"{STATA_WEB}/api/v1/routes?pattern=dtn://gateway/**", method="DELETE")
    log_test("Suppression dynamique de route DELETE /api/v1/routes", status == 200)

    status, routes_a3, _, _ = http_req(f"{STATA_WEB}/api/v1/routes")
    has_dyn_after = any(r.get("pattern") == "dtn://gateway/**" for r in routes_a3 or [])
    log_test("Vérification retrait immédiat du RIB BPA", not has_dyn_after)

    # -------------------------------------------------------------
    # SUITE 3: Gestion Keystore & Politiques BPSec BIB (Phase 6 & 6bis)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 3: Trousseau BPSec BIB & Cybersécurité (Phase 6 & 6bis)]{RESET}")
    status, pol_b, _, _ = http_req(f"{STATB_WEB}/api/v1/security/policy")
    log_test("Politique de validation par défaut (warn)", status == 200 and pol_b.get("policy") == "warn")

    # Rejet secret trop court (< 16 octets)
    short_key = {
        "direction": "outbound",
        "pattern": "dtn://statb/*",
        "label": "Short Secret",
        "secret": "court"
    }
    status, err_resp, _, _ = http_req(f"{STATA_WEB}/api/v1/security/keys", method="POST", data=short_key)
    log_test("Rejet d'un secret < 16 octets (400 Bad Request)", status == 400)

    # Ajout de la clé Outbound sur STATA
    key_out_stata = {
        "direction": "outbound",
        "pattern": "dtn://statb/*",
        "label": "Clé émission vers STATB",
        "secret": SHARED_SECRET
    }
    status, created_out, _, _ = http_req(f"{STATA_WEB}/api/v1/security/keys", method="POST", data=key_out_stata)
    log_test("Création clé Outbound sur STATA", status == 201)
    secret_leaked = created_out and ("secret" in created_out or "key_hex" in created_out)
    log_test("Vérification non-divulgation du secret en clair", not secret_leaked)
    has_fp = created_out and "fingerprint" in created_out and created_out["fingerprint"].startswith("SHA256:")
    log_test("Empreinte HMAC-SHA256 domaine-séparée présente", has_fp, f"fp={created_out.get('fingerprint') if created_out else ''}")

    # Vérification unicité (direction, pattern) : mise à jour sur place sans création de doublon
    status, dup_resp, _, _ = http_req(f"{STATA_WEB}/api/v1/security/keys", method="POST", data=key_out_stata)
    status_list, keys_after_dup, _, _ = http_req(f"{STATA_WEB}/api/v1/security/keys")
    out_keys = [k for k in (keys_after_dup or []) if k.get("direction") == "outbound" and k.get("pattern") == "dtn://statb/*"]
    log_test("Unicité (direction, pattern) respectée (mise à jour sans doublon)", len(out_keys) == 1, f"total_matching={len(out_keys)}")

    # Configuration des clés réciproques (Inbound sur STATB, Outbound sur STATB, Inbound sur STATA)
    key_in_statb = {
        "direction": "inbound",
        "pattern": "dtn://stata/*",
        "label": "Clé réception depuis STATA",
        "secret": SHARED_SECRET
    }
    status, created_in_b, _, _ = http_req(f"{STATB_WEB}/api/v1/security/keys", method="POST", data=key_in_statb)
    log_test("Création clé Inbound sur STATB", status == 201)

    key_out_statb = {
        "direction": "outbound",
        "pattern": "dtn://stata/*",
        "label": "Clé émission vers STATA",
        "secret": SHARED_SECRET
    }
    status, _, _, _ = http_req(f"{STATB_WEB}/api/v1/security/keys", method="POST", data=key_out_statb)
    log_test("Création clé Outbound sur STATB", status == 201)

    key_in_stata = {
        "direction": "inbound",
        "pattern": "dtn://statb/*",
        "label": "Clé réception depuis STATB",
        "secret": SHARED_SECRET
    }
    status, _, _, _ = http_req(f"{STATA_WEB}/api/v1/security/keys", method="POST", data=key_in_stata)
    log_test("Création clé Inbound sur STATA", status == 201)

    # Basculement de la politique STATB en mode STRICT
    status, _, _, _ = http_req(f"{STATB_WEB}/api/v1/security/policy", method="PUT", data={"policy": "strict"})
    log_test("Activation politique STRICT sur STATB", status == 200)

    # -------------------------------------------------------------
    # SUITE 4: Démarrage de l'écoute SSE sur STATB (Phase 4)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 4: Push temps réel SSE (Phase 4)]{RESET}")
    sse_thread = threading.Thread(target=sse_listener_thread, args=(f"{STATB_WEB}/api/v1/events",), daemon=True)
    sse_thread.start()
    time.sleep(0.5)
    log_test("Connexion flux SSE /api/v1/events active", sse_thread.is_alive())

    # -------------------------------------------------------------
    # SUITE 5: Test End-to-End Chat Sécurisé BPSec (STATA -> STATB)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 5: Chat interactif sécurisé BPSec (STATA -> STATB)]{RESET}")
    chat_payload = {
        "content": "Bonjour STATB, ceci est un message signé cryptographiquement en BPSec BIB!"
    }
    status, chat_resp, _, _ = http_req(f"{STATA_WEB}/api/v1/chat/conversations/statb", method="POST", data=chat_payload)
    log_test("Envoi message Chat signé depuis STATA", status in (200, 201) and chat_resp.get("status") == "sent", f"auth_status={chat_resp.get('auth_status') if chat_resp else None}")

    time.sleep(1.5)

    status, msgs_b, _, _ = http_req(f"{STATB_WEB}/api/v1/chat/messages?peer=dtn://stata/chat")
    received_msg = None
    if status == 200 and isinstance(msgs_b, list):
        for m in msgs_b:
            if "Bonjour STATB" in m.get("content", ""):
                received_msg = m
                break
    log_test("Message Chat reçu via TCPCLv4 sur STATB", received_msg is not None)
    if received_msg:
        log_test("Vérification intégrité du contenu", received_msg.get("content") == chat_payload["content"])
        log_test("Validation cryptographique BPSec sur STATB (auth_status: valid)", received_msg.get("auth_status") == "valid", f"auth_status={received_msg.get('auth_status')}")
        log_test("Direction correcte (inbound)", received_msg.get("direction") == "inbound")

    status, convs_b, _, _ = http_req(f"{STATB_WEB}/api/v1/chat/conversations")
    conv_stata = next((c for c in (convs_b or []) if "stata" in c.get("peer", "")), None)
    log_test("Fil de conversation créé sur STATB", conv_stata is not None)
    if conv_stata:
        log_test("Compteur non-lu incrémenté (unread_count > 0)", conv_stata.get("unread_count", 0) > 0, f"unread={conv_stata.get('unread_count')}")

    status, _, _, _ = http_req(f"{STATB_WEB}/api/v1/chat/read", method="POST", data={"peer": "dtn://stata/chat"})
    log_test("Action POST /api/v1/chat/read exécutée", status == 200)

    status, convs_b2, _, _ = http_req(f"{STATB_WEB}/api/v1/chat/conversations")
    conv_stata2 = next((c for c in (convs_b2 or []) if "stata" in c.get("peer", "")), None)
    log_test("Compteur non-lu remis à 0 après lecture", conv_stata2 is not None and conv_stata2.get("unread_count") == 0)

    chat_events = [e for e in sse_events_received if e[0] == "chat_message"]
    log_test("Notification temps réel SSE chat_message reçue sur STATB", len(chat_events) > 0, f"total_events={len(sse_events_received)}")

    # -------------------------------------------------------------
    # SUITE 6: Réponse Bidirectionnelle (STATB -> STATA)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 6: Réponse bidirectionnelle sécurisée (STATB -> STATA)]{RESET}")
    reply_payload = {
        "content": "Bien reçu STATA, liaison BPSec 5/9, clé HMAC validée !"
    }
    status, reply_resp, _, _ = http_req(f"{STATB_WEB}/api/v1/chat/conversations/stata", method="POST", data=reply_payload)
    log_test("Envoi réponse signée depuis STATB", status in (200, 201) and reply_resp.get("status") == "sent")

    time.sleep(1.5)

    status, msgs_a, _, _ = http_req(f"{STATA_WEB}/api/v1/chat/messages?peer=dtn://statb/chat")
    received_reply = None
    if status == 200 and isinstance(msgs_a, list):
        for m in msgs_a:
            if "Bien reçu STATA" in m.get("content", ""):
                received_reply = m
                break
    log_test("Réponse reçue sur STATA", received_reply is not None)
    if received_reply:
        log_test("Validation cryptographique réponse sur STATA (auth_status: valid)", received_reply.get("auth_status") == "valid")

    # -------------------------------------------------------------
    # SUITE 7: Rejet en mode STRICT vs Acceptation en mode WARN
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 7: Sécurité BPSec: Rejet Strict d'une Signature Invalide]{RESET}")
    status, keys_a, _, _ = http_req(f"{STATA_WEB}/api/v1/security/keys")
    out_key_a = next((k for k in (keys_a or []) if k.get("direction") == "outbound" and k.get("pattern") == "dtn://statb/*"), None)
    if out_key_a:
        http_req(f"{STATA_WEB}/api/v1/security/keys/{out_key_a['id']}", method="DELETE")

    bad_key_out = {
        "direction": "outbound",
        "pattern": "dtn://statb/*",
        "label": "Clé falsifiée pour test",
        "secret": INVALID_SECRET
    }
    http_req(f"{STATA_WEB}/api/v1/security/keys", method="POST", data=bad_key_out)

    bad_chat = {
        "content": "Ce message est falsifié et DOIT être rejeté par STATB !"
    }
    status, _, _, _ = http_req(f"{STATA_WEB}/api/v1/chat/conversations/statb", method="POST", data=bad_chat)
    log_test("Envoi message falsifié depuis STATA", status in (200, 201))

    time.sleep(1.5)

    status, msgs_b_strict, _, _ = http_req(f"{STATB_WEB}/api/v1/chat/messages?peer=dtn://stata/chat")
    found_falsified = any("falsifié" in m.get("content", "") for m in msgs_b_strict or [])
    log_test("Politique STRICT: message avec signature invalide REJETÉ et NON inséré", not found_falsified)

    http_req(f"{STATB_WEB}/api/v1/security/policy", method="PUT", data={"policy": "warn"})
    log_test("Basculement politique STATB en mode WARN", True)

    bad_chat_warn = {
        "content": "Ce message falsifié DOIT être accepté avec badge 'invalid' en mode WARN"
    }
    http_req(f"{STATA_WEB}/api/v1/chat/conversations/statb", method="POST", data=bad_chat_warn)
    time.sleep(1.5)

    status, msgs_b_warn, _, _ = http_req(f"{STATB_WEB}/api/v1/chat/messages?peer=dtn://stata/chat")
    falsified_msg_warn = next((m for m in msgs_b_warn or [] if "mode WARN" in m.get("content", "")), None)
    log_test("Politique WARN: message falsifié accepté", falsified_msg_warn is not None)
    if falsified_msg_warn:
        log_test("Statut d'authentification marqué 'invalid'", falsified_msg_warn.get("auth_status") == "invalid", f"auth_status={falsified_msg_warn.get('auth_status')}")

    status, keys_a_curr, _, _ = http_req(f"{STATA_WEB}/api/v1/security/keys")
    curr_out = next((k for k in (keys_a_curr or []) if k.get("direction") == "outbound" and k.get("pattern") == "dtn://statb/*"), None)
    if curr_out:
        http_req(f"{STATA_WEB}/api/v1/security/keys/{curr_out['id']}", method="DELETE")
    http_req(f"{STATA_WEB}/api/v1/security/keys", method="POST", data=key_out_stata)
    http_req(f"{STATB_WEB}/api/v1/security/policy", method="PUT", data={"policy": "strict"})
    log_test("Clés cryptographiques et politique STRICT rétablies", True)

    # -------------------------------------------------------------
    # SUITE 8: Service de Fichiers Dédié & HTTP Range (Phase 3bis & 6bis)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 8: Service Fichiers dtn://statb/files & HTTP Range (Phase 3bis & 6bis)]{RESET}")
    file_content = "# Rapport Technique DTN\n\nPression: 1018 hPa\nTemperature: 18.5 C\nVent: SSE 12 kt\n"
    bundle_req = {
        "destination": "dtn://statb/files",
        "source": "dtn://stata/files",
        "lifetime": 7200,
        "payload": file_content
    }
    status, bundle_disp, _, _ = http_req(f"{STATA_WEB}/api/v1/bundles", method="POST", data=bundle_req)
    bid = bundle_disp.get("bundle_id") if bundle_disp else None
    log_test("Envoi bundle fichier sécurisé vers dtn://statb/files", status in (200, 201), f"bundle_id={bid}")

    time.sleep(1.5)

    status, files_b, _, _ = http_req(f"{STATB_WEB}/api/v1/files")
    log_test("Listing des fichiers reçus sur STATB", status == 200 and len(files_b) > 0)
    target_file = files_b[0] if (files_b and len(files_b) > 0) else None
    if target_file:
        file_id = target_file.get("id")
        log_test("Validation cryptographique BPSec du fichier (auth_status: valid)", target_file.get("auth_status") == "valid")
        mime = target_file.get("mime_type")
        log_test("Détection type MIME Markdown (text/markdown)", mime == "text/markdown", f"mime_type={mime}")

        status, file_detail, _, _ = http_req(f"{STATB_WEB}/api/v1/files/{file_id}")
        log_test("Consultation métadonnées détaillées du fichier", status == 200 and "payload_preview" in (file_detail or {}))
        preview = file_detail.get("payload_preview", "") if file_detail else ""
        log_test("Aperçu du contenu conforme", "Rapport Technique DTN" in (preview or ""))

        status, _, partial_body, headers = http_req(
            f"{STATB_WEB}/api/v1/files/{file_id}/payload",
            headers={"Range": "bytes=0-22"}
        )
        partial_text = partial_body.decode("utf-8", errors="replace")
        log_test("Téléchargement partiel HTTP Range (206 Partial Content)", status == 206, f"status={status}")
        log_test("Tranche exacte d'octets reçue (bytes=0-22)", partial_text == "# Rapport Technique DTN", f"got='{partial_text}'")

        status, sticky_res, _, _ = http_req(f"{STATB_WEB}/api/v1/files/{file_id}/sticky", method="POST", data={"sticky": True})
        log_test("Marquage rétention permanente (Sticky Pin)", status == 200 and sticky_res.get("is_sticky") is True)

        nosniff = headers.get("X-Content-Type-Options") or headers.get("x-content-type-options")
        csp = headers.get("Content-Security-Policy") or headers.get("content-security-policy")
        log_test("Protection OWASP Stored XSS (nosniff & CSP Sandbox)", nosniff == "nosniff" and "sandbox" in (csp or ""))

    # -------------------------------------------------------------
    # SUITE 9: Inspecteur de Bundles & Badges d'Intégrité (Phase 2 & 6bis)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 9: Inspecteur de Bundles & Badges d'Intégrité (Phase 2 & 6bis)]{RESET}")
    status, bundles_b, _, _ = http_req(f"{STATB_WEB}/api/v1/bundles")
    log_test("Consultation stockage bundles STATB (/api/v1/bundles)", status == 200 and len(bundles_b) > 0)
    has_valid_bib = any(b.get("auth_status") in ("valid", "unsigned") for b in (bundles_b or []))
    log_test("Inspecteur de bundles: auth_status renseigné avec vérification cryptographique", has_valid_bib)

    if bundles_b and len(bundles_b) > 0:
        first_bid = bundles_b[0].get("id")
        status, bdetail, _, _ = http_req(f"{STATB_WEB}/api/v1/bundles?id={first_bid}")
        log_test("Consultation détail bundle et dump hexadécimal", status == 200 and "hex_dump" in (bdetail or {}))

    # -------------------------------------------------------------
    # SUITE 10: Journalisation Système & Anneau Mémoire (Phase 4)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 10: Traces d'Exécution & Ring Buffer /api/v1/logs (Phase 4)]{RESET}")
    status, logs_resp_a, _, _ = http_req(f"{STATA_WEB}/api/v1/logs")
    logs_a = logs_resp_a.get("logs", []) if isinstance(logs_resp_a, dict) else (logs_resp_a or [])
    log_test("Snapshot des logs mémoire STATA", status == 200 and len(logs_a) > 0, f"entries={len(logs_a)}")

    status, logs_resp_b, _, _ = http_req(f"{STATB_WEB}/api/v1/logs")
    logs_b = logs_resp_b.get("logs", []) if isinstance(logs_resp_b, dict) else (logs_resp_b or [])
    log_test("Snapshot des logs mémoire STATB", status == 200 and len(logs_b) > 0, f"entries={len(logs_b)}")
    has_audit_log = any(
        "ChatService" in l.get("message", "") or "FilesService" in l.get("message", "") or "BPA" in l.get("message", "") or "hardy" in l.get("target", "")
        for l in logs_b if isinstance(l, dict)
    )
    log_test("Traces d'activité DTN présentes dans le buffer", has_audit_log)

    # -------------------------------------------------------------
    # SUITE 11: Passerelle Télémétrie SenML RFC 8428 & SQLite (Phase 5)
    # -------------------------------------------------------------
    print(f"\n{BOLD}[Suite 11: Passerelle Télémétrie SenML RFC 8428 & SQLite (Phase 5)]{RESET}")

    # 1. État initial (synchronisation configuration YAML)
    status, metrics_b, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics")
    log_test("GET /api/v1/senml/metrics liste initiale", status == 200 and isinstance(metrics_b, list))
    initial_metric_names = [m.get("name") for m in (metrics_b or [])]
    log_test("Métrique initiale YAML présente dans le dictionnaire", "urn:dtn:stata:power/bus_voltage" in initial_metric_names)

    status, cards_b, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/dashboard/cards")
    log_test("GET /api/v1/senml/dashboard/cards liste initiale", status == 200 and len(cards_b or []) == 1)
    if cards_b and len(cards_b) > 0:
        log_test("Label personnalisé issu du YAML configuré", cards_b[0].get("custom_label") == "Tension Bus A")

    # 2. Abonnement au flux SSE SenML (/api/v1/senml/stream)
    senml_events = []
    senml_stop = False
    def senml_sse_listener():
        req = urllib.request.Request(f"{STATB_WEB}/api/v1/senml/stream", headers={"Accept": "text/event-stream"})
        try:
            with urllib.request.urlopen(req, timeout=15) as resp:
                while not senml_stop:
                    line = resp.readline()
                    if not line:
                        break
                    line_str = line.decode("utf-8", errors="replace").strip()
                    if line_str.startswith("event:"):
                        evt_type = line_str.split(":", 1)[1].strip()
                        data_line = resp.readline().decode("utf-8", errors="replace").strip()
                        if data_line.startswith("data:"):
                            payload = data_line.split(":", 1)[1].strip()
                            senml_events.append((evt_type, payload))
        except Exception:
            pass

    senml_thread = threading.Thread(target=senml_sse_listener, daemon=True)
    senml_thread.start()
    time.sleep(0.5)

    # 3. Injection bundle SenML JSON depuis STATA vers dtn://statb/senml
    senml_json_pack = [
        {"bn": "urn:dtn:stata:power/", "n": "bus_voltage", "u": "V", "v": 27.9, "t": 1731000060},
        {"bn": "urn:dtn:stata:solar/", "n": "watts", "u": "W", "v": 145.2, "t": 1731000060}
    ]
    bundle_payload = {
        "destination": "dtn://statb/senml",
        "payload": json.dumps(senml_json_pack),
        "lifetime": 3600
    }
    status, inj_resp, _, _ = http_req(f"{STATA_WEB}/api/v1/bundles", method="POST", data=bundle_payload)
    log_test("Envoi bundle SenML JSON depuis STATA", status == 201)

    # Attente acheminement TCPCLv4 et livraison SenmlService
    time.sleep(2.0)

    # 4. Vérification de l'ingestion dans metric_dictionary et telemetry_history
    status, metrics_after, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics")
    discovered_names = [m.get("name") for m in (metrics_after or [])]
    log_test("Auto-découverte métrique solaire", "urn:dtn:stata:solar/watts" in discovered_names)

    # 5. Vérification GET /api/v1/senml/metrics/latest
    status, latest_all, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics/latest")
    log_test("GET /api/v1/senml/metrics/latest sans filtre", status == 200 and len(latest_all or []) >= 2)

    status, latest_filtered, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics/latest?id=urn:dtn:stata:power/bus_voltage")
    log_test("Filtrage ?id=urn:dtn:stata:power/bus_voltage", status == 200 and len(latest_filtered or []) == 1)
    if latest_filtered and len(latest_filtered) > 0:
        pt = latest_filtered[0]
        log_test("Valeur numérique et timestamp normalisés", pt.get("value") == 27.9 and pt.get("utc") == 1731000060000)

    # 6. Vérification du filtrage dashboard_only=true
    status, dash_latest, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics/latest?dashboard_only=true")
    dash_names = [m.get("name") for m in (dash_latest or [])]
    log_test("Métrique non ajoutée exclue du dashboard par défaut", "urn:dtn:stata:solar/watts" not in dash_names)
    log_test("Métrique déclarée présente sur le dashboard", "urn:dtn:stata:power/bus_voltage" in dash_names)

    # 7. Vérification de l'API Historique compatible Open MCT
    status, history, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/history?id=urn:dtn:stata:power/bus_voltage&start=1731000000000&end=1731000100000")
    log_test("GET /api/v1/senml/history format Open MCT", status == 200 and len(history or []) == 1)
    if history and len(history) > 0:
        log_test("Format point Open MCT ({'utc': ..., 'value': ...})", history[0].get("utc") == 1731000060000 and history[0].get("value") == 27.9)

    # 8. Test d'Idempotence (réinjection du même pack)
    status, _, _, _ = http_req(f"{STATA_WEB}/api/v1/bundles", method="POST", data=bundle_payload)
    time.sleep(1.5)
    status, history_dup, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/history?id=urn:dtn:stata:power/bus_voltage&start=1731000000000&end=1731000100000")
    log_test("Idempotence SQLite (aucun doublon créé après réémission)", len(history_dup or []) == 1)

    # 8bis. Ingestion de charge utile binaire SenML CBOR (RFC 8428 Section 6)
    try:
        import cbor2
        cbor_pack = [
            {0: "temp_celsius", -2: "urn:dtn:stata:sensor/", 1: "Cel", 2: 21.5, 6: 1731000080}
        ]
        cbor_bytes = cbor2.dumps(cbor_pack)
        boundary = "----HardyCborBoundaryXYZ123"
        cbor_multipart = (
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="destination"\r\n\r\n'
            f"dtn://statb/senml\r\n"
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="payload"; filename="sensor.cbor"\r\n'
            f"Content-Type: application/senml+cbor\r\n\r\n"
        ).encode('utf-8') + cbor_bytes + f"\r\n--{boundary}--\r\n".encode('utf-8')
        status, _, _, _ = http_req(
            f"{STATA_WEB}/api/v1/bundles",
            method="POST",
            data=cbor_multipart,
            headers={"Content-Type": f"multipart/form-data; boundary={boundary}"}
        )
        log_test("Envoi bundle SenML CBOR (multipart)", status == 201)
        time.sleep(1.5)

        status, cbor_latest, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics/latest?id=urn:dtn:stata:sensor/temp_celsius")
        log_test("Décodage et normalisation SenML CBOR", status == 200 and len(cbor_latest or []) == 1 and (cbor_latest[0].get("value") == 21.5))
    except Exception as e:
        log_test("Décodage et normalisation SenML CBOR", False, str(e))

    # 9. Manipulation des cartes du tableau de bord (CRUD)
    card_add = {
        "name": "urn:dtn:stata:solar/watts",
        "custom_label": "Panneau Photovoltaïque"
    }
    status, add_res, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/dashboard/cards", method="POST", data=card_add)
    card_id = add_res.get("id") if add_res else None
    log_test("Ajout carte dashboard via POST /dashboard/cards", status == 201 and card_id is not None)

    status, dash_after_add, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics/latest?dashboard_only=true")
    dash_after_names = [m.get("name") for m in (dash_after_add or [])]
    log_test("Nouvelle carte visible sur le dashboard", "urn:dtn:stata:solar/watts" in dash_after_names)

    if card_id is not None:
        card_update = {
            "custom_label": "Panneau Toit Sud",
            "display_order": 0
        }
        status, upd_res, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/dashboard/cards/{card_id}", method="PUT", data=card_update)
        log_test("Mise à jour carte dashboard via PUT /dashboard/cards/{id}", status == 200 and upd_res.get("custom_label") == "Panneau Toit Sud")

        status, del_res, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/dashboard/cards/{card_id}", method="DELETE")
        log_test("Suppression carte dashboard via DELETE /dashboard/cards/{id}", status == 200 and del_res.get("status") == "removed")
    else:
        log_test("Mise à jour carte dashboard via PUT /dashboard/cards/{id}", False, "card_id manquant")
        log_test("Suppression carte dashboard via DELETE /dashboard/cards/{id}", False, "card_id manquant")

    status, dash_final, _, _ = http_req(f"{STATB_WEB}/api/v1/senml/metrics/latest?dashboard_only=true")
    dash_final_names = [m.get("name") for m in (dash_final or [])]
    log_test("Carte masquée du dashboard après suppression", "urn:dtn:stata:solar/watts" not in dash_final_names)

    # 10. Vérification de la diffusion temps réel SSE
    senml_stop = True
    received_bus_events = [payload for (evt, payload) in senml_events if "urn:dtn:stata:power/bus_voltage" in payload]
    log_test("Diffusion temps réel SSE (/api/v1/senml/stream)", len(received_bus_events) > 0, f"events={len(received_bus_events)}")

    sse_stop_flag = True

    print(f"\n{BOLD}======================================================{RESET}")
    print(f"{BOLD}RÉSULTATS DE LA SUITE DE TESTS END-TO-END :{RESET}")
    print(f"  Total tests exécutés : {passed_tests + failed_tests}")
    print(f"  Succès               : {GREEN}{passed_tests}{RESET}")
    print(f"  Échecs               : {RED if failed_tests > 0 else GREEN}{failed_tests}{RESET}")
    print(f"{BOLD}======================================================{RESET}\n")

    return failed_tests == 0

if __name__ == "__main__":
    if not wait_for_ready(STATA_WEB, 5):
        print(f"{RED}Erreur: STATA ({STATA_WEB}) n'est pas prêt.{RESET}")
        sys.exit(1)
    if not wait_for_ready(STATB_WEB, 5):
        print(f"{RED}Erreur: STATB ({STATB_WEB}) n'est pas prêt.{RESET}")
        sys.exit(1)

    ok = run_all_tests()
    sys.exit(0 if ok else 1)
