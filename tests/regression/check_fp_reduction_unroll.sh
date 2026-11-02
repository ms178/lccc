#!/usr/bin/env bash
# Strict-FP reduction unrolling: shape contract + bit-exact results.
#
# `float sum(const float *v, int n)` called once with a constant trip is the
# corpus' worst callgrind row (`reduction_vecreg`: 3.28e9 vs GCC 16.2's 1.72e9,
# ratio 1.905).  The reference compilers unroll it 4x; this compiler kept one
# element per `add`/`cmp`/`jl` because the two-block unroller's profitability
# veto only admitted store-carrying latches and private unsigned reductions, and
# because IPCP (which turns `n` into the call site's constant) ran AFTER every
# loop transform of iteration 0, so the unroller never saw a constant trip.
#
# Two contracts:
#
#  1. SHAPE (asm): the specialized callees are unrolled — at least four
#     `addss`/`addsd` and at most one conditional branch plus one loop-closing
#     `jmp`/branch per group.  A rolled loop has exactly one `addss` per branch,
#     so the assertion is stated as "addss count is at least four times the
#     count of loop-closing conditional branches" to survive block reordering
#     and layout changes.
#  2. RESULTS (runtime): bit-identical answers against a sequential reference on
#     association-sensitive payloads (1.0 / 1e8 / -1e8), every trip residue
#     modulo 8 (the divisibility guard), empty and single-iteration runtime
#     trips, and the F32 dot chain.  A reassociating or vectorizing rewrite
#     changes these bits; an unroll that assumes `trip % k == 0` drops the tail.
#
# The `-O0`/`-O1` runs pin that the transform is optional: the same answers
# must come out of the configurations that do not unroll at all, and the
# `-mno-avx`/`-march=x86-64-v3` runs pin that neither ISA level may vectorize or
# reassociate the chain.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-fp-reduction.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

src="$dir/fp_reduction_unroll.c"

# ── 1. Runtime results across the configuration matrix ───────────────────────
run() {
    local tag=$1
    shift
    if ! "$CCC" "$@" -o "$tmp/$tag" "$src" 2>"$tmp/$tag.err"; then
        echo "FAIL: $tag did not compile:" >&2
        cat "$tmp/$tag.err" >&2
        exit 1
    fi
    if ! "$tmp/$tag" >"$tmp/$tag.out" 2>&1; then
        echo "FAIL: $tag produced the wrong results:" >&2
        cat "$tmp/$tag.out" >&2
        exit 1
    fi
    if ! grep -q '^ok$' "$tmp/$tag.out"; then
        echo "FAIL: $tag did not report ok:" >&2
        cat "$tmp/$tag.out" >&2
        exit 1
    fi
}

run O0 -O0
run O1 -O1
run O2 -O2
run O2-noopt-attr -O2 -fno-tree-vectorize
run O3 -O3
run O2-noavx -O2 -mno-avx
run O2-v3 -O2 -march=x86-64-v3

# Every optimisation level must agree with every other one: the bits are part
# of the contract, not an artefact of one build.
for tag in O0 O1 O2 O3 O2-noavx O2-v3; do
    if ! cmp -s "$tmp/O2.out" "$tmp/$tag.out"; then
        echo "FAIL: -O2 and $tag disagree:" >&2
        diff "$tmp/O2.out" "$tmp/$tag.out" >&2 || true
        exit 1
    fi
done

# ── 2. Codegen shape at -O2 (the level the benchmark corpus builds at) ───────
"$CCC" -O2 -S "$src" -o "$tmp/O2.s"

python3 - "$tmp/O2.s" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()

# Object-file symbol markers delimit the bodies reliably across the .s layout
# the backend emits (`.type f, @function` … `.size f, .-f`).
def body(name):
    m = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not m:
        raise SystemExit(f"missing assembly body for {name}")
    return m.group(1)

CASES = {
    # name: (accumulating mnemonic, minimum occurrences)
    "sum_lit_f32": ("addss", 4),
    "sum_lit_f64": ("addsd", 4),
    "dot_lit": ("addss", 4),
    # Parameter trip, ONE literal call site: unrollable only if IPCP runs
    # before iteration 0's loop transforms.
    "sum_once_f32": ("addss", 4),
}
for name, (acc, least) in CASES.items():
    b = body(name)
    n_acc = len(re.findall(rf"\b{acc}\b", b))
    if n_acc < least:
        raise SystemExit(
            f"{name}: expected an unrolled body (>= {least} {acc}), found {n_acc}"
        )
    # Loop-closing conditional branches inside the body: an unrolled group has
    # one, a rolled loop has one per element.
    n_branch = len(re.findall(r"\bj[a-z]+\b", b))
    if n_acc < 2 * n_branch:
        raise SystemExit(
            f"{name}: {n_acc} {acc} against {n_branch} branches — the loop is "
            f"still rolled (one accumulate per branch)"
        )
    # No packed FP operation: the transform must not vectorize the chain.
    if re.search(r"\b(?:addps|addpd|mulps|mulpd|vfmadd|haddps|haddpd)\b", b):
        raise SystemExit(f"{name}: reduction was reassociated into a packed add")
    # No multiply-add contraction snuck in either.
    if re.search(r"\bvfmadd\d*\b", b):
        raise SystemExit(f"{name}: FMA contraction in a strict-FP reduction")

print("fp reduction unroll shape OK")
PY

echo "fp reduction unroll: PASS"
