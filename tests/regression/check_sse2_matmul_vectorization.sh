#!/usr/bin/env bash
# Baseline-x86-64 (SSE2) vectorization of the packed multiply-add loop shape.
#
# THE DEFECT THIS PINS
# --------------------
# The 2-wide matmul arm existed and the backend could already lower it without
# FMA (`mulpd`/`addpd`, one rounding per operation, exactly the scalar
# semantics), but the arm was refused whenever the target had no FMA3 unit:
#
#     } else if !fma_transform_allowed(neon, fp_contract)
#         && (neon || use_sse2 || !x86_64_vector_intrinsics_available())
#
# so `-O2` on the default x86-64 target -- no `-march`, the most common
# configuration there is -- emitted a scalar loop for `C[j] += A * B[j]` while
# GCC 12+ vectorizes it at -O2 (`-ftree-vectorize` is on there).  On the
# development host (Xeon @2.6 GHz, no `-march`) that was 10.00 ms against
# GCC's 6.96 ms for a 256x256 f64 product; with the arm enabled it is 6.63 ms.
#
# The licence to contract (`-ffp-contract`) is withdrawn only when the chosen
# lowering actually fuses, which on x86 means FMA3 *and* 256-bit vectors (the
# `LCCC_FORCE_SSE2`-on-AVX2 case goes through `vfmadd231pd`).  Everything
# checked below follows from that distinction.
#
# WHAT IS CHECKED
# ---------------
#   1. `-O2` at the default target: the loop is packed (`mulpd`/`addpd`) and
#      contains no FMA, and it prints the same 17 digits as GCC.
#   2. `-O2 -ffp-contract=off` at the default target: still packed, still no
#      FMA, still GCC-identical -- the non-fused lowering is not a contraction,
#      so withdrawing the licence cannot forbid it.
#   3. `-O2 -march=x86-64-v3`: the fused form (`vfmadd231pd`) appears, i.e.
#      the wide target keeps the faster op.
#   4. `-O2 -march=x86-64-v3 -ffp-contract=off`: no FMA -- where the lowering
#      really does fuse, the licence is still required.
#   5. `-O2 -march=x86-64-v3 -ffp-contract=off` with `LCCC_FORCE_SSE2=1`: no
#      FMA either.  This is the one x86 configuration whose 2-wide lowering is
#      routed through the 4-lane `vfmadd231pd`; the gate must not hand it the
#      arm just because the arm is named "SSE2".  Staying scalar is the
#      intended answer there (the knob is a debugging override, and the only
#      2-wide lowering available on that target fuses), so this case checks the
#      absence of FMA only.
#   6. `-O2 -mno-sse` refuses the function (no silent scalar fallback that
#      would need x87, which this compiler does not lower).
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
GCC=${GCC_BIN:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cd "$td"

cat > mm.c <<'EOF'
#include <stdio.h>
#define N 32
static double A[N][N], B[N][N], C[N][N];
int main(void) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) {
            A[i][j] = (double)(i + j) / N;
            B[i][j] = (double)(i * j + 1) / N;
        }
    for (int i = 0; i < N; i++)
        for (int k = 0; k < N; k++)
            for (int j = 0; j < N; j++)
                C[i][j] += A[i][k] * B[k][j];
    printf("%.17g\n", C[N / 2][N / 2] + C[0][0]);
    return 0;
}
EOF

PACKED_RE='(^|[[:space:]])[[:space:]]*(mulpd|addpd|vfmadd|vmulpd|vaddpd)'
FMA_RE='(^|[[:space:]])(vfmadd|vfmsub|vfnmadd|vfnmsub)[a-z0-9]*[[:space:]]'
fail=0
note() { printf 'FAIL: %s\n' "$1" >&2; fail=1; }

# Reference: the scalar semantics every packed form must reproduce.
"$GCC" -O2 -o mm-gcc mm.c
want=$("./mm-gcc")

packed_or_fail() { # file, label
    grep -qE "$PACKED_RE" "$1" || note "$2: the loop was not packed (no mulpd/addpd)"
}
no_fma_or_fail() { # file, label
    if grep -qE "$FMA_RE" "$1"; then
        note "$2: fused into an FMA"
    fi
}

# 1. Default target, -O2: packed, unfused, GCC-identical.
"$CCC" -O2 -S mm.c -o base.s
"$CCC" -O2 -o mm-base mm.c
packed_or_fail base.s "-O2 (default target)"
no_fma_or_fail base.s "-O2 (default target)"
[ "$("./mm-base")" = "$want" ] || note "-O2 (default target): output differs from GCC ($("./mm-base") vs $want)"

# 2. -ffp-contract=off at the default target: same code, by the same argument.
"$CCC" -O2 -ffp-contract=off -S mm.c -o off.s
"$CCC" -O2 -ffp-contract=off -o mm-off mm.c
packed_or_fail off.s "-O2 -ffp-contract=off (default target)"
no_fma_or_fail off.s "-O2 -ffp-contract=off (default target)"
[ "$("./mm-off")" = "$want" ] || note "-O2 -ffp-contract=off: output differs from GCC"

# 3. v3: the fused form is the point of the wide target (positive control for
#    the FMA regex used by checks 4 and 5).
"$CCC" -O2 -march=x86-64-v3 -S mm.c -o v3.s
"$CCC" -O2 -march=x86-64-v3 -o mm-v3 mm.c
grep -qE "$FMA_RE" v3.s || note "-O2 -march=x86-64-v3: no FMA (positive control failed)"
[ "$("./mm-v3")" = "$want" ] || note "-O2 -march=x86-64-v3: output differs from GCC"

# 4. v3 with the licence withdrawn.
"$CCC" -O2 -march=x86-64-v3 -ffp-contract=off -S mm.c -o v3off.s
"$CCC" -O2 -march=x86-64-v3 -ffp-contract=off -o mm-v3off mm.c
# Withdrawing the licence must cost the fusion (`vmulpd`+`vaddpd`), never the
# vectorization: a scalar fallback here would be a silent 2x.
packed_or_fail v3off.s "-O2 -march=x86-64-v3 -ffp-contract=off"
no_fma_or_fail v3off.s "-O2 -march=x86-64-v3 -ffp-contract=off"
[ "$("./mm-v3off")" = "$want" ] || note "-O2 -march=x86-64-v3 -ffp-contract=off: output differs from GCC"

# 5. Forced 2-wide on an AVX2+FMA target: still no FMA without the licence.
LCCC_FORCE_SSE2=1 "$CCC" -O2 -march=x86-64-v3 -ffp-contract=off -S mm.c -o v3force.s
LCCC_FORCE_SSE2=1 "$CCC" -O2 -march=x86-64-v3 -ffp-contract=off -o mm-v3force mm.c
no_fma_or_fail v3force.s "LCCC_FORCE_SSE2 + -march=x86-64-v3 + -ffp-contract=off"
[ "$("./mm-v3force")" = "$want" ] || note "LCCC_FORCE_SSE2 + v3 + -ffp-contract=off: output differs from GCC"

# 6. FP with SIMD forbidden is a hard error, not x87 fallback.
if "$CCC" -O2 -mno-sse -S mm.c -o nosse.s 2>/dev/null; then
    note "-mno-sse accepted a floating-point function"
fi

if [ "$fail" -ne 0 ]; then
    exit 1
fi
printf 'sse2 matmul vectorization: ok\n'
