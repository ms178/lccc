#!/usr/bin/env bash
# FP-contraction parity gate (GCC: -O0/-O1 never fuse, -O2 does).
#
# THE DEFECT THIS PINS
# --------------------
# LCCC's C default is `-ffp-contract=fast` (matching GCC's *flag* default), and
# it applied at every optimization level.  Measured on gcc 14.2, GCC never
# contracts `a*b + c` into an FMA at -O0 or -O1 -- not even with
# `-march=x86-64-v3` and not even under `-ffast-math`; the fusion appears at
# -O2.  So `lccc -O0` and `gcc -O0` printed different numbers for the same FP
# program, which is how 26 `miscompile_gen_fp_stress_*` reproducers were filed
# against this compiler while both results were conforming (the real defect was
# the level mismatch, not the arithmetic).
#
# WHAT IS CHECKED
# ---------------
#   1. -O0 and -O1 emit NO fma mnemonic for `a*b + c` (GCC parity), while
#      -O2 does (positive control: a gate that never sees an FMA cannot
#      prove the rule is specific to the low levels).
#   2. An explicit `-ffp-contract=fast` at -O0 DOES contract, and an explicit
#      `-ffp-contract=off` at -O2 does NOT: the flag still wins.
#   3. Numeric agreement with GCC at -O0/-O1 (identical stdout, 17 digits)
#      on a cancellation-sensitive chain, and at -O2 with the ISA-matched GCC
#      arm (`-march=x86-64-v3` = LCCC's documented default target).
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
GCC=${GCC_BIN:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cd "$td"

cat > fma.c <<'EOF'
double fuse(double a, double b, double c) { return a * b + c; }
EOF
cat > chain.c <<'EOF'
#include <stdio.h>
int main(void) {
    volatile double v0 = 384.9218040120, c4 = 94.333300881796674;
    double s = v0;
    s = s * c4 + v0; s = s * c4 + v0; s = s * c4 + v0;
    s = s * c4 + v0; s = s * c4 + v0; s = s * c4 + v0;
    printf("%.17g\n", s);
    return 0;
}
EOF

FMA_RE='(^|[[:space:]])(vfmadd|vfmsub|vfnmadd|vfnmsub|fmadd|fmsub)[a-z0-9]*[[:space:]]'
fail=0
note() { printf 'FAIL: %s\n' "$1" >&2; fail=1; }

# 1. level matrix on the emitted assembly. FMA is post-x86-64 (opt-in), so the
#    default build must contain none at any level; the positive controls use
#    the explicit v3 profile.
"$CCC" -O0 -S fma.c -o o0.s
"$CCC" -O1 -S fma.c -o o1.s
"$CCC" -O2 -S fma.c -o o2.s
"$CCC" -O2 -march=x86-64-v3 -S fma.c -o o2v3.s
grep -cE "$FMA_RE" o0.s >/dev/null && note "-O0 fused a*b+c into an FMA (GCC does not)"
grep -cE "$FMA_RE" o1.s >/dev/null && note "-O1 fused a*b+c into an FMA (GCC does not)"
grep -cE "$FMA_RE" o2.s >/dev/null && note "default -O2 emitted an FMA (post-baseline ISA leaked)"
grep -cE "$FMA_RE" o2v3.s >/dev/null || note "-march=x86-64-v3 -O2 did NOT emit an FMA (positive control lost)"

# 2. explicit flags win at any level (under the v3 grant that makes FMA legal).
"$CCC" -O0 -march=x86-64-v3 -ffp-contract=fast -S fma.c -o o0fast.s
"$CCC" -O2 -march=x86-64-v3 -ffp-contract=off -S fma.c -o o2off.s
grep -cE "$FMA_RE" o0fast.s >/dev/null || note "-O0 -ffp-contract=fast did not contract (flag ignored)"
grep -cE "$FMA_RE" o2off.s >/dev/null && note "-O2 -ffp-contract=off still contracted (flag ignored)"

# 3. numeric agreement with matched flags on both sides: default -O0/-O1/-O2
#    (no FMA anywhere), then -O2 under the v3 grant (both sides fuse).
check_num() { # check_num <lccc-flags> <gcc-flags> <label>
    "$CCC" $1 chain.c -o lccc.bin
    "$GCC" $2 chain.c -o gcc.bin
    ours=$(./lccc.bin); theirs=$(./gcc.bin)
    [ "$ours" = "$theirs" ] || note "$3 numeric mismatch: lccc=$ours gcc=$theirs"
}
for lvl in -O0 -O1 -O2; do check_num "$lvl" "$lvl" "default $lvl"; done
check_num "-O2 -march=x86-64-v3" "-O2 -march=x86-64-v3" "v3 -O2"

[ "$fail" = 0 ] || { echo 'fp-contract parity: FAIL'; exit 1; }
echo 'fp-contract parity: PASS (default: no FMA at any -O level; v3 grant: -O2 fuses; flags honored; numbers agree)'
