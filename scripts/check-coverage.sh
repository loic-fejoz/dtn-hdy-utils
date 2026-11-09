#!/usr/bin/env bash
# scripts/check-coverage.sh
# Runs source-based test code coverage across the workspace using cargo-llvm-cov.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# Default parameters
EXCLUDES=(
    "--exclude" "hardy-hqfbp-cla-fuzz"
    "--exclude" "hardy-ltp-cla-fuzz"
    "--exclude" "hardy-ltp-fuzz"
)

HTML_MODE=false
LCOV_MODE=false
JSON_MODE=false
OUTPUT_PATH=""
EXTRA_ARGS=()

show_help() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS] [-- [ADDITIONAL_CARGO_TEST_ARGS]]

Runs workspace code coverage using cargo-llvm-cov.

Options:
    --html                  Generate an HTML report in target/llvm-cov/html/
    --open                  Generate an HTML report and open it in the default browser
    --lcov                  Generate an LCOV tracefile (default: ./lcov.info)
    --json                  Generate a JSON coverage report
    --output-path <PATH>    Specify custom output file path for --lcov or --json
    --summary-only          Print only the total coverage summary table
    -h, --help              Show this help message and exit

Examples:
    $0                      # Print coverage table to console
    $0 --summary-only       # Quick summary
    $0 --html               # Generate HTML report
    $0 --lcov               # Generate lcov.info for CI
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --html)
            HTML_MODE=true
            shift
            ;;
        --open)
            HTML_MODE=true
            EXTRA_ARGS+=("--open")
            shift
            ;;
        --lcov)
            LCOV_MODE=true
            shift
            ;;
        --json)
            JSON_MODE=true
            shift
            ;;
        --output-path)
            OUTPUT_PATH="$2"
            shift 2
            ;;
        --summary-only)
            EXTRA_ARGS+=("--summary-only")
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        --)
            shift
            EXTRA_ARGS+=("$@")
            break
            ;;
        *)
            EXTRA_ARGS+=("$1")
            shift
            ;;
    esac
done

# Check if cargo-llvm-cov is installed
if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "✗ 'cargo-llvm-cov' is not installed or not in PATH."
    echo ""
    echo "To install cargo-llvm-cov and required llvm-tools component, run:"
    echo "  rustup component add llvm-tools-preview"
    echo "  curl -sSfL https://github.com/taiki-e/cargo-llvm-cov/releases/latest/download/cargo-llvm-cov-x86_64-unknown-linux-musl.tar.gz | tar -xz -C ~/.cargo/bin"
    echo "  # or: cargo install cargo-llvm-cov --locked"
    exit 1
fi

echo "=== Running Workspace Code Coverage (cargo-llvm-cov) ==="
echo "Repository: ${REPO_ROOT}"

if [ "${HTML_MODE}" = true ]; then
    echo "▶ Generating HTML coverage report in target/llvm-cov/html/..."
    cargo llvm-cov --workspace "${EXCLUDES[@]}" --html "${EXTRA_ARGS[@]}"
    echo "✓ HTML report generated at: file://${REPO_ROOT}/target/llvm-cov/html/index.html"
elif [ "${LCOV_MODE}" = true ]; then
    OUT_FILE="${OUTPUT_PATH:-lcov.info}"
    echo "▶ Generating LCOV coverage report: ${OUT_FILE}..."
    cargo llvm-cov --workspace "${EXCLUDES[@]}" --lcov --output-path "${OUT_FILE}" "${EXTRA_ARGS[@]}"
    echo "✓ LCOV report saved to: ${OUT_FILE}"
elif [ "${JSON_MODE}" = true ]; then
    OUT_FILE="${OUTPUT_PATH:-coverage.json}"
    echo "▶ Generating JSON coverage report: ${OUT_FILE}..."
    cargo llvm-cov --workspace "${EXCLUDES[@]}" --json --output-path "${OUT_FILE}" "${EXTRA_ARGS[@]}"
    echo "✓ JSON report saved to: ${OUT_FILE}"
else
    cargo llvm-cov --workspace "${EXCLUDES[@]}" "${EXTRA_ARGS[@]}"
fi
