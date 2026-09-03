#!/usr/bin/env bash
# Codegen gate for the guarded-clz zero-guard elimination.
#
# `x ? __builtin_clz(x) : 32` must collapse onto the Clz intrinsic (whose IR
# semantics already define Clz(0) == 32). Before the fix, codegen emitted the
# intrinsic's own zero fix-up AND a second select materialization
# (`movq $32, %rdx; cmovneq %rax, %rdx`) — a fully dead tail. The gate checks
# the function body for the select residue and for the bsr core.
# Runtime semantics are covered differentially by guarded_clz_ctz_ternary.c.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/c.c" <<'EOF'
int lz(unsigned x) { return x ? __builtin_clz(x) : 32; }
EOF

"$CCC" -O2 -S "$td/c.c" -o "$td/c.s"
body=$(sed -n '/^lz:/,/^\.size[[:space:]]*lz/p' "$td/c.s")

if ! grep -qE '\bbsrl?\b|\blzcntl\b' <<<"$body"; then
    echo "FAIL: lz() should contain a bsr (or lzcnt) core; got:" >&2
    echo "$body" >&2
    exit 1
fi
# Select residue from the pre-fix duplicated zero guard: a second materialized
# `movq $32` + cmov (and the double test of the operand) must be gone.
if grep -qE '\bcmov' <<<"$body" || grep -qE 'movq[[:space:]]+\$32,' <<<"$body"; then
    echo "FAIL: lz() still contains the redundant zero-guard select tail:" >&2
    echo "$body" >&2
    exit 1
fi
n_test=$(grep -cE '\btest[blqw]?\b' <<<"$body" || true)
if [ "$n_test" -gt 1 ]; then
    echo "FAIL: lz() tests its operand more than once (doubled zero guard):" >&2
    echo "$body" >&2
    exit 1
fi
echo "OK clz_zero_guard_fold"
