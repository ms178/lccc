#!/usr/bin/env bash
# GCC 16.2 per-vectorizer tri-state + serial OpenMP mode regression gate.
#
# Five gcc.c-torture/compile cases (pr86636.c, pr86637-1.c, pr86637-3.c,
# pr99787-1.c, pr89235.c) used to fail at the DRIVER, before parsing: the CLI
# hard-rejected `-f{no-,}tree-{loop,slp}-vectorize` with
# "per-vectorizer control is unsupported" and `-fopenmp` with
# "OpenMP lowering and runtime linkage are not implemented".
#
# GCC 16.2 actually has THREE independent switches. Measured with
# `gcc-16 (GCC) 16.2.0 -O2 -fopt-info-vec-optimized` on a loop + SLP shape:
#
#   -fno-tree-vectorize                       -> loop OFF, slp OFF
#   -fno-tree-vectorize -ftree-loop-vectorize-> loop ON,  slp OFF
#   -ftree-loop-vectorize -fno-tree-vectorize-> loop ON,  slp OFF
#   -fno-tree-loop-vectorize -ftree-vectorize-> loop OFF, slp ON
#   -fno-tree-loop-vectorize -ftree-loop-vectorize -> loop ON (last wins)
#
# i.e. an explicit per-family switch overrides the master switch in EITHER
# order, and the last spelling within a family wins. A single `bool` cannot
# express that. This gate asserts the policy has an OBSERVABLE artifact effect
# (SIMD vs scalar loop), not merely that the flags are accepted.
#
# `-fopenmp` is implemented as OpenMP's SINGLE-THREAD SERIAL execution model:
# all `#pragma omp` directives are no-ops (one team of one thread), which is a
# conforming OpenMP execution. gcc-16 16.2.0 emits ZERO GOMP_* calls for the
# `declare simd` shape in pr89235.c. The observable effect is the `_OPENMP`
# predefined macro (202111, measured `gcc-16 -fopenmp -dM -E`). Runtime entry
# points are resolved by the linker, not fabricated.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC_I686=${CCC_I686:-./target/fastbuild/lccc-i686}
GCC=${GCC:-gcc-16}
command -v "$GCC" >/dev/null 2>&1 || GCC=gcc
[ -x "$CCC" ] || { echo "check_vectorizer_openmp_policy: SKIP ($CCC not built)"; exit 0; }

tmp=${TMPDIR:-/tmp}/lccc-vectorizer-policy.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

pass=0
fail=0
check() { # check <label> <condition-result>
    if [ "$2" -eq 0 ]; then
        pass=$((pass + 1))
    else
        printf 'FAIL: %s\n' "$1"
        fail=$((fail + 1))
    fi
}

# ── 1. Every spelling is accepted, on both ABIs ───────────────────────────
cat >"$tmp/flags.c" <<'C'
#pragma omp declare simd
int f(int x) { return x * 3 + 1; }
int a[16], b[16];
void g(int n) { for (int i = 0; i < n; ++i) a[i] = b[i] + 3; }
C
for cc in "$CCC" "$CCC_I686"; do
    [ -x "$cc" ] || continue
    for flags in \
        "-fno-tree-vectorize" \
        "-ftree-vectorize" \
        "-fno-tree-loop-vectorize" \
        "-ftree-loop-vectorize" \
        "-fno-tree-slp-vectorize" \
        "-ftree-slp-vectorize" \
        "-fno-tree-vectorize -ftree-loop-vectorize" \
        "-ftree-loop-vectorize -fno-tree-vectorize" \
        "-fno-tree-loop-vectorize -ftree-vectorize" \
        "-fno-tree-slp-vectorize -ftree-slp-vectorize" \
        "-fopenmp" \
        "-fno-openmp" \
        "-fopenmp -fno-openmp" \
        "-fopenmp -fsave-optimization-record -ftree-parallelize-loops=2 -fno-tree-vectorize --param ggc-min-expand=0"
    do
        # shellcheck disable=SC2086
        if "$cc" -O2 $flags -c "$tmp/flags.c" -o "$tmp/out.o" >"$tmp/out.log" 2>&1 \
           && ! grep -q 'error:' "$tmp/out.log"; then
            check "$(basename "$cc") $flags" 0
        else
            printf 'FAIL: %s %s -> %s\n' "$(basename "$cc")" "$flags" "$(head -2 "$tmp/out.log")"
            check "$(basename "$cc") $flags" 1
        fi
    done
done

# ── 2. The tri-state policy changes the ARTIFACT (not just acceptance) ─────
cat >"$tmp/vec.c" <<'C'
void vp(int *a, const int *b, int n) { for (int i = 0; i < n; i++) a[i] = b[i] + 3; }
C
simd_count() { # simd_count <flags...>
    # shellcheck disable=SC2086
    "$CCC" -O2 "$@" -S "$tmp/vec.c" -o - 2>/dev/null \
        | grep -ciE 'xmm|ymm|movdqu|paddd' || true
}
base=$(simd_count)
no_loop=$(simd_count -fno-tree-loop-vectorize)
no_all=$(simd_count -fno-tree-vectorize)
explicit_loop=$(simd_count -fno-tree-vectorize -ftree-loop-vectorize)
if [ "$base" -gt "$no_loop" ] && [ "$no_loop" -eq "$no_all" ] \
   && [ "$explicit_loop" -eq "$base" ]; then
    check "tri-state artifact effect (base=$base no_loop=$no_loop explicit=$explicit_loop)" 0
else
    printf 'FAIL: tri-state artifact effect: base=%s no_loop=%s no_all=%s explicit=%s\n' \
        "$base" "$no_loop" "$no_all" "$explicit_loop"
    check 'tri-state artifact effect' 1
fi

# ── 3. probe-fopenmp-serial: _OPENMP is defined only when enabled ──────────
openmp_define=$(echo | "$CCC" -fopenmp -dM -E -x c - 2>/dev/null | grep '^#define _OPENMP ' || true)
no_openmp_define=$(echo | "$CCC" -dM -E -x c - 2>/dev/null | grep '^#define _OPENMP ' || true)
last_wins=$(echo | "$CCC" -fopenmp -fno-openmp -dM -E -x c - 2>/dev/null | grep '^#define _OPENMP ' || true)
if [ "$openmp_define" = "#define _OPENMP 202111" ] \
   && [ -z "$no_openmp_define" ] && [ -z "$last_wins" ]; then
    check 'probe-fopenmp-serial: _OPENMP=202111 and last-wins' 0
else
    printf 'FAIL: _OPENMP on="%s" off="%s" last_wins="%s"\n' \
        "$openmp_define" "$no_openmp_define" "$last_wins"
    check 'probe-fopenmp-serial' 1
fi

# ── 4. Serial OpenMP execution is semantically identical ──────────────────
cat >"$tmp/serial.c" <<'C'
#include <stdio.h>
static int arr[8];
#pragma omp declare simd
static int scale(int x) { return x * 2 + 1; }
int main(void)
{
    int sum = 0;
    for (int i = 0; i < 8; ++i) { arr[i] = scale(i); sum += arr[i]; }
    if (sum != 64) { printf("FAIL sum=%d, expected 64\n", sum); return 1; }
    return 0;
}
C
if "$CCC" -O2 -fopenmp -c "$tmp/serial.c" -o "$tmp/serial.o" 2>"$tmp/serial.err" \
   && ! grep -q 'error:' "$tmp/serial.err" \
   && "$GCC" -o "$tmp/serial" "$tmp/serial.o" 2>/dev/null \
   && "$tmp/serial"; then
    check 'serial OpenMP declare simd runtime' 0
else
    printf 'FAIL: serial OpenMP runtime: %s\n' "$(cat "$tmp/serial.err" 2>/dev/null)"
    check 'serial OpenMP declare simd runtime' 1
fi

printf 'check_vectorizer_openmp_policy: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
