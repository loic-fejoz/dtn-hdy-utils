#!/usr/bin/env bash
# ==============================================================================
# scripts/install-hooks.sh
# Installs local Git hooks to enforce quality and security guardrails.
# ==============================================================================

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

HOOK_PATH=".git/hooks/pre-commit"

cat << 'EOF' > "$HOOK_PATH"
#!/usr/bin/env bash
# Pre-commit hook enforcing code quality, panic-free code, and secret scanning
set -euo pipefail

echo "======================================================"
echo "▶ [pre-commit] Running security & quality guardrails..."
echo "======================================================"

exec ./scripts/check-security.sh --strict
EOF

chmod +x "$HOOK_PATH"
echo "✓ Pre-commit hook successfully installed at $HOOK_PATH"
