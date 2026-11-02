#!/usr/bin/env bash
# Registry-aware, byte-exact compiler correctness oracle. Keep this entry point
# stable for CI and existing callers. LCCC_BIN, GCC_BIN and OPT are unchanged.
# Unlike shell command substitution, the Python gate preserves trailing LF and
# NUL bytes, and treats matching crashes as failures rather than agreement.
set -euo pipefail
REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
exec python3 "$REPO/scripts/check_benchmark_outputs.py" "$@"
