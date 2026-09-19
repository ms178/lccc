#!/usr/bin/env bash
# Two-block partial unroller + half-wide SLP pairs red-team gate.
#
#   1. The battery must compile at -O2 -march=x86-64-v3 and run clean.
#   2. Tri-config differential: byte-identical stdout under (a) everything
#      on, (b) CCC_NO_TWO_BLOCK_UNROLL=1 CCC_NO_BB_SLP=1 (the rolled
#      scalar reference of the same compiler), and (c) gcc -O2
#      -march=x86-64-v3 (the independent oracle).
#   3. Unroller kill-switch differential: CCC_NO_TWO_BLOCK_UNROLL=1 with
#      SLP left ON must also match (the SLP must not depend on the
#      unroller's presence for correctness).
#   4. SSE2-baseline parity: -march=x86-64 (no AVX2) must run clean — the
#      half-wide pairs and the VEX forms must fail closed, never
#      miscompile, when the ISA refuses them.
#   5. Codegen contracts (scoped asm checks on the -S output):
#      - d4_schedule_pairs (the sha256 message-schedule shape): at least
#        3 vmovq pair loads and at least 6 VEX dword ops, and ZERO legacy
#        SSE packed-ARITHMETIC ops inside the function (the seed-path
#        gather deliberately uses the SSE2-exact movd/punpckldq pair: it
#        must pack on every x86-64 baseline — no SSE4.1 dependency — and
#        it executes once per group where any transition consideration is
#        amortized to zero; the contract grep scopes to the per-iteration
#        packed arithmetic, which is all-VEX by design);
#      - f1_hash_chain: the folded constant-key compare (cmp{l,q} $imm,
#        off(%reg)) is present — the immediate-source load fold;
#      - the A1 miscompile-class kernel: at least one pair of adjacent
#        dword stores folded into a 64-bit vmovq/movq store somewhere in
#        the battery's unrolled loops is NOT required (profitability
#        declines are legitimate); the contract is the d4 shape above.
#   6. Loop-vec Max-reduction kill-switch differential: CCC_NO_LOOP_VEC_MAX=1
#      (the scalar loop reference of the same transform) must match — the
#      e1_find_max kernel pins the gate's correctness both ways (WO-3).
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/two_block_unroll_redteam.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

march=""
if "$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null; then
    march="-march=x86-64-v3"
fi

expect="two_block_unroll_redteam: all pass (0 fails)"

# ── 1. runtime, everything on ────────────────────────────────────────────
"$ccc" -O2 $march "$src" -o "$td/rt_on"
out_on=$("$td/rt_on")
if [ "$out_on" != "$expect" ]; then
    echo "FAIL: two_block_unroll_redteam runtime (all on): $out_on"
    exit 1
fi

# ── 2. tri-config differential ───────────────────────────────────────────
CCC_NO_TWO_BLOCK_UNROLL=1 CCC_NO_BB_SLP=1 "$ccc" -O2 $march "$src" -o "$td/rt_off"
out_off=$("$td/rt_off")
if [ "$out_off" != "$out_on" ]; then
    echo "FAIL: two_block_unroll_redteam ON vs OFF differential"
    echo "  on : $out_on"
    echo "  off: $out_off"
    exit 1
fi
if command -v gcc >/dev/null 2>&1; then
    gcc -O2 $march "$src" -o "$td/rt_gcc"
    out_gcc=$("$td/rt_gcc")
    if [ "$out_gcc" != "$out_on" ]; then
        echo "FAIL: two_block_unroll_redteam lccc vs gcc differential"
        echo "  lccc: $out_on"
        echo "  gcc : $out_gcc"
        exit 1
    fi
fi

# ── 3. unroller kill-switch differential (SLP on) ────────────────────────
CCC_NO_TWO_BLOCK_UNROLL=1 "$ccc" -O2 $march "$src" -o "$td/rt_nounroll"
out_nounroll=$("$td/rt_nounroll")
if [ "$out_nounroll" != "$out_on" ]; then
    echo "FAIL: two_block_unroll_redteam unroll kill-switch differential"
    echo "  on      : $out_on"
    echo "  nounroll: $out_nounroll"
    exit 1
fi

# ── 4. SSE2-baseline parity (fail-closed ISA discipline) ─────────────────
"$ccc" -O2 "$src" -o "$td/rt_sse2"
out_sse2=$("$td/rt_sse2")
if [ "$out_sse2" != "$out_on" ]; then
    echo "FAIL: two_block_unroll_redteam SSE2-baseline differential"
    echo "  v3  : $out_on"
    echo "  sse2: $out_sse2"
    exit 1
fi

# ── 5. loop-vec Max-reduction kill-switch differential (WO-3) ────────────
CCC_NO_LOOP_VEC_MAX=1 "$ccc" -O2 $march "$src" -o "$td/rt_nomax"
out_nomax=$("$td/rt_nomax")
if [ "$out_nomax" != "$out_on" ]; then
    echo "FAIL: two_block_unroll_redteam Max-reduction kill-switch differential"
    echo "  on   : $out_on"
    echo "  nomax: $out_nomax"
    exit 1
fi

# ── 6. codegen contracts ─────────────────────────────────────────────────
"$ccc" -O2 $march -S "$src" -o "$td/rt.s"

# WO-6: the asm ranges are BOUNDED on both ends — the `.size` directive OR
# the next function label, whichever comes first. A bare
# `/^d4_schedule_pairs:/,/\.size/` range degrades to EOF when the `.size`
# line is ever absent (a different emitter, a stripped dump), inflating the
# counts with the REST of the file's instructions — a false PASS. Ending on
# the next global label too can only ever SHRINK the range (lccc's internal
# labels are .L-prefixed), so the failure direction stays fail-closed.
scoped_fn_asm() {
    awk -v fn="$1" '
        BEGIN { inrange = 0 }
        $0 ~ "^" fn ":" { inrange = 1 }
        inrange && /^[A-Za-z_][A-Za-z0-9_]*:$/ && $0 !~ ("^" fn ":") { inrange = 0 }
        inrange { print }
        inrange && $0 ~ ("\\.size[[:space:]]+" fn) { inrange = 0 }
    ' "$2"
}

d4=$(scoped_fn_asm d4_schedule_pairs "$td/rt.s")
# grep -c exits 1 on zero matches; the contract checks below own the verdict.
vmovq=$(printf '%s\n' "$d4" | grep -c 'vmovq' || true)
vexops=$(printf '%s\n' "$d4" | grep -cE '^ +v(padd|psrl|psll|pxor|por)' || true)
legacy=$(printf '%s\n' "$d4" | grep -cE '^ +(padd|psrl|psll|pxor|por)[a-z]* ' || true)
if [ "$vmovq" -lt 3 ] || [ "$vexops" -lt 6 ]; then
    echo "FAIL: d4_schedule_pairs lost the half-wide pair form (vmovq=$vmovq vex=$vexops)"
    exit 1
fi
if [ "$legacy" -ne 0 ]; then
    echo "FAIL: d4_schedule_pairs mixes legacy SSE with VEX ($legacy legacy ops)"
    exit 1
fi

f1=$(scoped_fn_asm f1_hash_chain "$td/rt.s")
if ! printf '%s\n' "$f1" | grep -qE ' +cmp(l|q) \$[0-9-]+, [0-9-]*\(%r'; then
    echo "FAIL: f1_hash_chain constant-key compare did not fold (no cmp \$imm, off(%reg))"
    exit 1
fi

echo "ok: two_block_unroll_redteam (runtime, tri-config, kill-switch, SSE2 baseline, Max kill-switch, asm contracts)"
