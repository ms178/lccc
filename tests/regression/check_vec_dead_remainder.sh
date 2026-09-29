#!/usr/bin/env bash
# ============================================================================
# check_vec_dead_remainder.sh — ZERO-REM-1: provably-dead vectorizer remainder
# loops are omitted, and the omission is only applied where it is sound.
#
# The map vectorizer emits a packed loop plus a SCALAR MIRROR that (a) runs the
# `n % W` tail elements and (b) is the fallback target of the runtime
# dependence guards.  When the trip count is a compile-time constant the packed
# body covers exactly, the mirror cannot iterate, and it is now dropped.
#
# Three contracts, pinned separately because each can fail alone:
#
#   1. CORRECTNESS. `vec_dead_remainder.c` must agree with the GCC oracle on
#      stdout AND exit status at every optimization level, for the SSE2
#      (x86-64) and AVX2 (x86-64-v3) baselines, and for the trailing-edge
#      neighbours of the transform (n = W, n = W+1, n = 2W, dynamic n,
#      may-alias streams, escaping counters, nested loops).  It must ALSO agree
#      with lccc compiled with CCC_NO_MAP_VEC=1 — the vectorizer's own scalar
#      path — because a correct program compiled two ways by the same compiler
#      is the cheapest differential oracle that isolates this transform.
#
#   2. THE MECHANISM FIRES, AND ONLY WHERE IT MAY. `check_vec_remainder_shapes.py`
#      counts the loops of every shape in `vec_dead_remainder_shapes.c`
#      (strongly connected components, not "backward branches": block layout
#      makes the latter unreliable).  Exact multiples must have ONE loop,
#      non-multiples TWO, may-alias TWO — and a shape that stops vectorizing
#      fails instead of silently passing with one scalar loop.
#
#   3. NO ARCHITECTURE IS LEFT BEHIND. i686 has no map-vectorizer lowering; the
#      corpus must still compile, link and produce the oracle's answer there.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_vec_dead_remainder: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_vec_dead_remainder: no gcc oracle" >&2; exit 1; }

corpus=$here/vec_dead_remainder.c
# The shape fixture lives in a subdirectory on purpose: `run_regression.py`
# compiles every top-level `tests/regression/*.c` as a standalone program and
# requires a `main`, and this one is an assembly-shape census -- it is only
# ever compiled to `-S` and inspected, never linked or run.
shapes=$here/vec_shapes/vec_dead_remainder_shapes.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-zerorem.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# ---------------------------------------------------------------- contract 1
note "contract 1: correctness against the GCC oracle and against CCC_NO_MAP_VEC"
for arch in x86-64 x86-64-v3; do
    "$GCC" -O2 "-march=$arch" "$corpus" -o "$work/ref-$arch"
    timeout 60 "$work/ref-$arch" > "$work/expected-$arch" \
        || bad "$arch: gcc oracle exited non-zero"
    for opt in -O0 -O1 -O2 -O3 -Os; do
        flags=($opt "-march=$arch")
        if ! "$CCC" "${flags[@]}" "$corpus" -o "$work/run" 2> "$work/err"; then
            bad "$arch $opt: compile failed: $(head -1 "$work/err")"
            continue
        fi
        if ! timeout 60 "$work/run" > "$work/actual"; then
            bad "$arch $opt: run failed"
            continue
        fi
        cmp -s "$work/expected-$arch" "$work/actual" \
            || bad "$arch $opt: output differs from gcc"
        # Same compiler, vectorizer disabled: isolates the transform.
        if ! env CCC_NO_MAP_VEC=1 "$CCC" "${flags[@]}" "$corpus" -o "$work/scalar" 2> "$work/err"; then
            bad "$arch $opt: scalar compile failed: $(head -1 "$work/err")"
            continue
        fi
        timeout 60 "$work/scalar" > "$work/scalar-out" \
            || bad "$arch $opt: scalar run failed"
        cmp -s "$work/expected-$arch" "$work/scalar-out" \
            || bad "$arch $opt: output differs with CCC_NO_MAP_VEC=1"
    done
done
note "  (default -march, -O2)"
"$GCC" -O2 "$corpus" -o "$work/ref-default"
timeout 60 "$work/ref-default" > "$work/expected-default"
"$CCC" -O2 "$corpus" -o "$work/run" && timeout 60 "$work/run" > "$work/actual" \
    && cmp -s "$work/expected-default" "$work/actual" \
    || bad "default -O2: output differs from gcc"

# ---------------------------------------------------------------- contract 2
note "contract 2: loop census of every shape (dead mirror omitted / kept)"
asm_files=()
for arch in x86-64 x86-64-v3; do
    asm=$work/shapes-$arch.s
    if ! "$CCC" -O2 "-march=$arch" -S "$shapes" -o "$asm" 2> "$work/err"; then
        bad "$arch: could not compile the shape corpus: $(head -1 "$work/err")"
        continue
    fi
    asm_files+=("$asm")
done
if [ ${#asm_files[@]} -gt 0 ]; then
    python3 "$here/check_vec_remainder_shapes.py" "${asm_files[@]}" || fail=1
fi

# ---------------------------------------------------------------- contract 3
note "contract 3: i686 (no map-vectorizer lowering) still matches the oracle"
if "$GCC" -O2 -m32 "$corpus" -o "$work/ref32" 2> /dev/null; then
    timeout 60 "$work/ref32" > "$work/expected32" || bad "i686: gcc oracle exited non-zero"
    for opt in -O0 -O2; do
        if ! "$CCC" $opt -m32 "$corpus" -o "$work/run32" 2> "$work/err"; then
            bad "i686 $opt: compile failed: $(head -1 "$work/err")"
            continue
        fi
        timeout 60 "$work/run32" > "$work/actual32" || bad "i686 $opt: run failed"
        cmp -s "$work/expected32" "$work/actual32" \
            || bad "i686 $opt: output differs from gcc"
    done
else
    note "  SKIP: no 32-bit gcc multilib on this host"
fi

if [ "$fail" -eq 0 ]; then
    echo "check_vec_dead_remainder: PASS"
else
    echo "check_vec_dead_remainder: FAIL" >&2
fi
exit "$fail"
