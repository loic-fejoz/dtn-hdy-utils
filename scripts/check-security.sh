#!/usr/bin/env bash
# ==============================================================================
# scripts/check-security.sh
#
# Unified security and code quality checker for dtn-hdy-utils.
# Runs secret scanning, dependency vulnerability auditing, formatting, and linting.
#
# Usage:
#   ./scripts/check-security.sh            # Standard run (warns if optional tools missing)
#   ./scripts/check-security.sh --strict   # Strict run (fails if any tool is missing or fails)
# ==============================================================================

set -euo pipefail

# ANSI color codes
BOLD="\033[1m"
GREEN="\033[1;32m"
YELLOW="\033[1;33m"
RED="\033[1;31m"
BLUE="\033[1;34m"
RESET="\033[0m"

STRICT_MODE=false
if [[ "${1:-}" == "--strict" || "${1:-}" == "--ci" ]]; then
    STRICT_MODE=true
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo -e "${BOLD}${BLUE}=== Running Security & Code Quality Guardrails ===${RESET}"
echo -e "Repository root: ${REPO_ROOT}"
echo -e "Strict mode:     ${STRICT_MODE}\n"

FAILURES=0

run_check() {
    local name="$1"
    shift
    echo -e "${BOLD}▶ [1/5] Checking ${name}...${RESET}"
    if "$@"; then
        echo -e "${GREEN}✓ ${name} passed.${RESET}\n"
    else
        echo -e "${RED}✗ ${name} failed!${RESET}\n"
        FAILURES=$((FAILURES + 1))
    fi
}

# ------------------------------------------------------------------------------
# 1. Rust Code Formatting
# ------------------------------------------------------------------------------
echo -e "${BOLD}▶ [1/5] Checking Rust code formatting (cargo fmt)...${RESET}"
if cargo fmt --check; then
    echo -e "${GREEN}✓ Rust code formatting is compliant.${RESET}\n"
else
    echo -e "${RED}✗ Rust formatting check failed. Run 'cargo fmt' to fix.${RESET}\n"
    FAILURES=$((FAILURES + 1))
fi

# ------------------------------------------------------------------------------
# 2. Rust Clippy Linter
# ------------------------------------------------------------------------------
echo -e "${BOLD}▶ [2/5] Checking Rust static analysis & linter (cargo clippy)...${RESET}"
if cargo clippy --workspace --all-targets -- -D warnings; then
    echo -e "${GREEN}✓ Cargo clippy passed with zero warnings.${RESET}\n"
else
    echo -e "${RED}✗ Cargo clippy detected errors or warnings.${RESET}\n"
    FAILURES=$((FAILURES + 1))
fi

# ------------------------------------------------------------------------------
# 3. Secret Scanning (Gitleaks)
# ------------------------------------------------------------------------------
echo -e "${BOLD}▶ [3/5] Checking for hardcoded secrets (gitleaks)...${RESET}"
GITLEAKS_BIN="$(command -v gitleaks || true)"
if [ -z "$GITLEAKS_BIN" ] && [ -x "$HOME/.cargo/bin/gitleaks" ]; then
    GITLEAKS_BIN="$HOME/.cargo/bin/gitleaks"
fi

if [ -n "$GITLEAKS_BIN" ]; then
    # Scan git history if git repo, otherwise scan directory
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        if "$GITLEAKS_BIN" git --config .gitleaks.toml --verbose; then
            echo -e "${GREEN}✓ No secrets detected in git history.${RESET}\n"
        else
            echo -e "${RED}✗ Gitleaks detected potential secrets or credentials!${RESET}\n"
            FAILURES=$((FAILURES + 1))
        fi
    else
        if "$GITLEAKS_BIN" dir . --config .gitleaks.toml --verbose; then
            echo -e "${GREEN}✓ No secrets detected in directory scan.${RESET}\n"
        else
            echo -e "${RED}✗ Gitleaks detected potential secrets!${RESET}\n"
            FAILURES=$((FAILURES + 1))
        fi
    fi
else
    echo -e "${YELLOW}⚠ Gitleaks binary not found.${RESET}"
    echo -e "  To install: curl -sSfL https://github.com/gitleaks/gitleaks/releases/download/v8.24.0/gitleaks_8.24.0_linux_x64.tar.gz | tar -xz -C ~/.cargo/bin gitleaks"
    if [ "$STRICT_MODE" = true ]; then
        echo -e "${RED}✗ Strict mode enabled: missing gitleaks is a failure.${RESET}\n"
        FAILURES=$((FAILURES + 1))
    else
        echo -e "${YELLOW}  Skipping secret scan (non-strict mode).${RESET}\n"
    fi
fi

# ------------------------------------------------------------------------------
# 4. Dependency Vulnerability Audit (Cargo Audit)
# ------------------------------------------------------------------------------
echo -e "${BOLD}▶ [4/5] Checking Rust dependencies for vulnerabilities (cargo audit)...${RESET}"
CARGO_AUDIT_BIN="$(command -v cargo-audit || true)"
if [ -z "$CARGO_AUDIT_BIN" ] && [ -x "$HOME/.cargo/bin/cargo-audit" ]; then
    CARGO_AUDIT_BIN="$HOME/.cargo/bin/cargo-audit"
fi

if [ -n "$CARGO_AUDIT_BIN" ] || cargo audit --version >/dev/null 2>&1; then
    if cargo audit; then
        echo -e "${GREEN}✓ Cargo audit passed with no unhandled vulnerabilities.${RESET}\n"
    else
        echo -e "${RED}✗ Cargo audit detected vulnerable dependencies!${RESET}\n"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo -e "${YELLOW}⚠ cargo-audit not found.${RESET}"
    echo -e "  To install: cargo install cargo-audit --locked"
    if [ "$STRICT_MODE" = true ]; then
        echo -e "${RED}✗ Strict mode enabled: missing cargo-audit is a failure.${RESET}\n"
        FAILURES=$((FAILURES + 1))
    else
        echo -e "${YELLOW}  Skipping cargo audit (non-strict mode).${RESET}\n"
    fi
fi

# ------------------------------------------------------------------------------
# 5. Web Frontend Dependency Audit (pnpm / npm audit)
# ------------------------------------------------------------------------------
echo -e "${BOLD}▶ [5/5] Checking Web frontend dependencies for vulnerabilities...${RESET}"
if [ -d "dtn-hdy-ham/web" ]; then
    # Protect against restricted runtime / sandbox permissions on XDG directories
    if ! (mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/.test-$$" 2>/dev/null && rmdir "${XDG_DATA_HOME:-$HOME/.local/share}/.test-$$" 2>/dev/null); then
        export XDG_DATA_HOME="/tmp/xdg-data"
    fi
    if ! (mkdir -p "${XDG_RUNTIME_DIR:-/tmp}/.test-$$" 2>/dev/null && rmdir "${XDG_RUNTIME_DIR:-/tmp}/.test-$$" 2>/dev/null); then
        export XDG_RUNTIME_DIR="/tmp"
    fi

    PNPM_RUN=""
    for cand in "pnpm" "/usr/bin/pnpm" "npx" "/usr/bin/npx"; do
        if command -v "$cand" >/dev/null 2>&1 && "$cand" --version >/dev/null 2>&1; then
            if [[ "$cand" == *"npx"* ]]; then
                PNPM_RUN="$cand --yes pnpm"
            else
                PNPM_RUN="$cand"
            fi
            break
        fi
    done

    if [ -n "$PNPM_RUN" ]; then
        if $PNPM_RUN --dir dtn-hdy-ham/web audit; then
            echo -e "${GREEN}✓ Frontend web dependencies are secure.${RESET}\n"
        else
            echo -e "${RED}✗ Frontend web dependencies have known vulnerabilities!${RESET}\n"
            FAILURES=$((FAILURES + 1))
        fi
    else
        echo -e "${YELLOW}⚠ Neither pnpm nor npx found.${RESET}"
        if [ "$STRICT_MODE" = true ]; then
            echo -e "${RED}✗ Strict mode enabled: missing web audit tool is a failure.${RESET}\n"
            FAILURES=$((FAILURES + 1))
        else
            echo -e "${YELLOW}  Skipping frontend dependency audit (non-strict mode).${RESET}\n"
        fi
    fi
else
    echo -e "${YELLOW}ℹ dtn-hdy-ham/web directory not found, skipping.${RESET}\n"
fi

# ------------------------------------------------------------------------------
# Final Summary
# ------------------------------------------------------------------------------
echo -e "${BOLD}======================================================${RESET}"
if [ "$FAILURES" -eq 0 ]; then
    echo -e "${GREEN}${BOLD}✓ ALL SECURITY & QUALITY CHECKS PASSED.${RESET}"
    exit 0
else
    echo -e "${RED}${BOLD}✗ $FAILURES CHECK(S) FAILED. Please review the output above.${RESET}"
    exit 1
fi
