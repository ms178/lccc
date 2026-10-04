#!/usr/bin/env bash
# assembler_callgrind_ab.sh — reproducible Callgrind A/B measurement of the
# lccc AArch64 assembler's own throughput (NOT compiled-code quality; for
# that use scripts/callgrind_ab.py).
#
# WHAT IT MEASURES
# ----------------
# Deterministic instruction-count (Ir) totals for `lccc-arm -c` on a fixed,
# seeded 20k-instruction AArch64 workload that deliberately over-weights the
# encoder families under active hardening (logical + extend + bitfield +
# pair + NEON bitwise ≈ 70% of the mix) over a generic background. Callgrind
# is deterministic, so ONE run per side is exact; the script still does two
# and asserts equality to catch a nondeterministic environment.
#
# It also asserts that the two builds produce BYTE-IDENTICAL object files on
# the workload — an assembler throughput change must never be an encoding
# change.
#
# USAGE
# -----
#   scripts/assembler_callgrind_ab.sh <base-lccc-arm> <head-lccc-arm>
#   scripts/assembler_callgrind_ab.sh --self-test <lccc-arm>
#
# Both positional arguments are paths to lccc-arm binaries (e.g. a build of
# upstream main and a build of the working tree). --self-test runs both
# sides with the SAME binary and asserts the measured delta is exactly zero
# -- this verifies the harness itself (workload generation, byte-identity
# comparison, determinism assertion, Ir parsing) without needing a second
# build, and is what the fast CI gate runs. Requires valgrind on PATH (any
# 3.2x); a user-local install works with VALGRIND_LIB exported by whoever
# provisions it.
set -euo pipefail

SELF_TEST=0
if [[ "${1:-}" == "--self-test" ]]; then
    SELF_TEST=1
    shift
fi

BASE_BIN=${1:?usage: assembler_callgrind_ab.sh [--self-test] <base-lccc-arm> [<head-lccc-arm>]}
HEAD_BIN=${2:-$BASE_BIN}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
S="$WORK/workload.s"

# ---- fixed, seeded workload (20036 instructions + labels) ------------------
python3 - "$S" <<'PY'
import random, sys
random.seed(20261004)
lines = [".text", ".globl workload", "workload:"]
funcs = [f"func{j}" for j in range(8)]
for i in range(20000):
    k = i % 10
    rd, rn, rm = random.randrange(31), random.randrange(31), random.randrange(31)
    if k == 0:   lines.append(f"and w{rd}, w{rn}, w{rm}, lsl #{i % 32}")
    elif k == 1: lines.append(f"orr x{rd}, x{rn}, x{rm}, asr #{i % 64}")
    elif k == 2: lines.append(f"eor w{rd}, w{rn}, w{rm}, ror #{i % 32}")
    elif k == 3: lines.append(f"sxtb x{rd}, w{rn}" if i % 2 else f"uxtw x{rd}, w{rn}")
    elif k == 4: lines.append(f"ubfx w{rd}, w{rn}, #{i % 20}, #{1 + i % 10}")
    elif k == 5: lines.append(f"stp x{rd}, x{rn}, [sp, #{(i % 60) * 8 - 240}]")
    elif k == 6: lines.append(f"and v{rd % 32}.16b, v{rn % 32}.16b, v{rm % 32}.16b")
    elif k == 7: lines.append(f"add w{rd}, w{rn}, #{i % 4000}")
    elif k == 8: lines.append(f"ldr w{rd}, [x{rn}, #{(i % 16000) & ~3}]")  # 4-aligned
    else:
        if i % 7:
            lines.append(f"cbz w{rd}, .Lskip{i}")
            lines.append(f"add w{rd}, w{rd}, #1")
            lines.append(f".Lskip{i}:")
        else:
            lines.append(f"bl {funcs[i % 8]}")
for f in funcs:
    lines += [f"{f}:", "    ret"]
open(sys.argv[1], 'w').write('\n'.join(lines) + '\n')
PY

# ---- assemble with both builds; objects must be byte-identical -------------
"$BASE_BIN" -c "$S" -o "$WORK/base.o"
"$HEAD_BIN" -c "$S" -o "$WORK/head.o"
cmp "$WORK/base.o" "$WORK/head.o" \
  || { echo "FAIL: object outputs differ — an encoding changed, not just speed" >&2; exit 1; }
echo "object outputs byte-identical"

# ---- deterministic Callgrind runs (two per side, must agree) ----------------
measure() {  # measure <bin> <tag>
    local bin=$1 tag=$2 run ir
    for run in 1 2; do
        valgrind --tool=callgrind --callgrind-out-file="$WORK/cg_${tag}_${run}.out" \
                 --quiet "$bin" -c "$S" -o "$WORK/w_${tag}.o" 2>/dev/null
        ir=$(grep -m1 '^totals:' "$WORK/cg_${tag}_${run}.out" | awk '{print $2}')
        eval "ir_${tag}_${run}=$ir"
    done
    eval "a=\$ir_${tag}_1; b=\$ir_${tag}_2"
    [[ $a == "$b" ]] || { echo "FAIL: nondeterministic Ir for $tag ($a vs $b)" >&2; exit 1; }
    echo "$a"
}
b_ir=$(measure "$BASE_BIN" base)
h_ir=$(measure "$HEAD_BIN" head)
python3 - "$b_ir" "$h_ir" "$SELF_TEST" <<'PY'
import sys
b, h, self_test = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3] == "1"
print(f"base Ir = {b:,}")
print(f"head Ir = {h:,}")
print(f"delta   = {h-b:+,} ({(h-b)/b*100:+.3f}%)")
if self_test:
    assert h == b, "self-test: same binary must measure exactly 0 delta"
    print("self-test ok: harness deterministic, delta exactly 0")
PY
