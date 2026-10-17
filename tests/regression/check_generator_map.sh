#!/usr/bin/env bash
# Keep the stable hosted/local gate entry point; one fail-closed Python driver
# owns runtime parity and per-kernel firing/refusal validation.
set -euo pipefail
exec python3 "$(dirname "${BASH_SOURCE[0]}")/check_generator_map.py" "$@"
