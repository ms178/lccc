#!/usr/bin/env bash
# The data-driven frontend and frame-addressing gate family, in one CI step.
#
# The three rule gates (`check_frontend_ternary_common_type.sh`,
# `check_frontend_preprocessor_rules.sh`, `check_frontend_duplicate_definition.sh`)
# are separate CI steps of their own; what is left is the machinery that needs a
# corpus or a generator rather than a hand-written case:
#
#   1. scripts/frontend_differential_audit.py -- the 41-case corpus against
#      gcc and clang: `-E` token streams, run-time values at every -O level,
#      and accept/reject agreement for diagnostics.
#   2. scripts/giant_frame_asm_audit.py -- static addressing audit of the
#      giant-frame corpus at every level (imm32 encoding, anchor legality,
#      extent), with a self-test that proves the audit rejects each violation
#      it claims to catch.
#   3. scripts/frontend_expression_fuzz.py -- a seeded random-expression sweep,
#      the part of the frontend a fixed corpus cannot cover.
#
# Every step is run with the caller's CCC, so the same script serves a local
# build (CCC=target/fastbuild/lccc) and hosted CI (CCC=target/fastbuild/lccc
# from the artifact).
set -uo pipefail

CCC=${CCC:-target/fastbuild/lccc}
FUZZ_CASES=${FUZZ_CASES:-24}
here=$(cd "$(dirname "$0")/../.." && pwd)
cd "$here" || exit 2

status=0
run() {
    local name=$1
    shift
    printf '\n== %s\n' "$name"
    if "$@"; then
        return 0
    fi
    printf 'FAILED: %s\n' "$name" >&2
    status=1
    return 0
}

run "frontend differential audit (gcc + clang)" \
    python3 scripts/frontend_differential_audit.py --lccc "$CCC"
run "frame addressing audit self-test" \
    python3 scripts/giant_frame_asm_audit.py --self-test
run "frame addressing audit" \
    python3 scripts/giant_frame_asm_audit.py --lccc "$CCC"
run "randomised expression differential" \
    python3 scripts/frontend_expression_fuzz.py --lccc "$CCC" --cases "$FUZZ_CASES"

if [ "$status" -ne 0 ]; then
    printf '\nfrontend-gate-family: FAILED\n' >&2
    exit 1
fi
printf '\nfrontend-gate-family: ok\n'
