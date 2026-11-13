#!/usr/bin/env bash
# Gate: the explicit 256-bit FMA3 map family must stay register-homed.
#
# The defect this gate pins (measured on the fastbuild tree of 2026-11-12):
# `_mm256_fmadd_ps(a,b,c)` compiled to 28 instructions in a 408-byte frame --
# three 32-byte dynamically-realign'd parameter homes, three reloads, a
# 32-byte result home and a reload -- because the `m3` table entry
# (`FmaPs132v256`) takes an ADDRESS per operand and writes through a
# `dest_ptr`.  GCC 16.2 emits `vfmadd132ps %ymm2,%ymm1,%ymm0; ret`.
#
# A second, independent shape family: `_mm256_loadu_ps(p)` lowers to a
# POINTER-result intrinsic (`LoaduPs256` writes 32 bytes to a scratch home)
# plus a register read of that home.  The read is now re-pointed at the load's
# source when it is a whole-register copy of exactly the bytes the load wrote,
# so `_mm256_fmadd_ps(_mm256_loadu_ps(p), b, c)` is one `vfmadd132ps (%rdi)`
# instead of `subq $72` + `vmovdqu` + spill + `vfmadd132ps 40(%rsp)` + reload
# (8-9 instructions).  Those shapes carry the same budgets below.
#
# Two independent assertions, because they fail independently:
#
#   1. HOMING.  Each shape's emitted body stays inside an instruction budget
#      AND contains no frame pointer realignment (`andq $-32`) -- that idiom
#      is how `emit_alloca_addr_to` reaches an over-aligned slot, so its
#      absence proves no 32-byte home took part.  The budget for `fma_ps` /
#      `fma_pd` also forbids a stack frame outright (2 instructions in GCC,
#      measured; the pre-fix tree emitted 28 and 21).
#   2. VALUES.  The same file runs and compares every lane bit-exactly with
#      `fmaf`/`fma` from libm.  The register-form path is a different emitter
#      than the memory form and FMA3 is destructive, so an operand aliasing
#      the destination is only observable at run time.
#
# Non-vacuity: the budgets are the pre-fix count divided by more than six, so
# the gate cannot pass on the tree that motivated it; `--expect-defect`
# inverts the requirement and is used with a stand-in compiler to prove the
# gate can fail.
#
# Usage: check_vector_fma_homing.sh [--lccc PATH] [--reference CC]
#                                  [--expect-defect]
set -euo pipefail

LCCC=""; REFERENCE=""; EXPECT_DEFECT=0
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --expect-defect) EXPECT_DEFECT=1; shift ;;
        -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "check_vector_fma_homing: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CC="${LCCC:-$ROOT/target/fastbuild/lccc}"
FIXTURE="$ROOT/tests/simd/vector_fma_homing.c"
[ -x "$CC" ] || { echo "FAIL: lccc binary '$CC' is not executable" >&2; exit 1; }
[ -r "$FIXTURE" ] || { echo "FAIL: fixture '$FIXTURE' is missing" >&2; exit 1; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
echo "check_vector_fma_homing: 256-bit FMA3 register homing"

# --- 1. values -------------------------------------------------------------
if ! "$CC" -O2 -w -mavx2 -mfma "$FIXTURE" -lm -o "$WORK/run" 2>"$WORK/cc.log"; then
    echo "FAIL: cannot build the value probe" >&2
    cat "$WORK/cc.log" >&2
    exit 1
fi
if "$WORK/run" >"$WORK/out.txt" 2>&1; then
    echo "  ok  $(cat "$WORK/out.txt")"
else
    echo "FAIL: the FMA shapes disagree with libm" >&2
    cat "$WORK/out.txt" >&2
    exit 1
fi

# --- 2. detector self-test -------------------------------------------------
# A volatile operand legitimately keeps the legacy address form (the register
# path refuses volatile objects), so its emitted body MUST still show the
# over-aligned frame idiom.  That makes the detector below non-vacuous: it is
# run against text that really carries the shape it forbids.
cat > "$WORK/legacy.c" <<'EOF'
#include <immintrin.h>
__m256 legacy_probe(__m256 a, __m256 b, volatile __m256 *p) {
    return _mm256_fmadd_ps(*p, b, a);
}
EOF
"$CC" -O2 -w -mavx2 -mfma -S "$WORK/legacy.c" -o "$WORK/legacy.s" 2>/dev/null \
    || { echo "FAIL: cannot emit the detector self-test" >&2; exit 1; }
legacy_body=$(awk '/^legacy_probe:/{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$WORK/legacy.s")
if grep -qE 'andq[[:space:]]+\$-32' <<<"$legacy_body"; then
    echo "  ok  detector self-test: the legacy address form is recognised"
else
    echo "FAIL: the detector self-test emitted no over-aligned frame idiom," >&2
    echo "      so the homing assertions below could not fail.  Body:" >&2
    printf '%s\n' "$legacy_body" | sed 's/^/        /' >&2
    exit 1
fi

# --- 3. homing -------------------------------------------------------------
"$CC" -O2 -w -mavx2 -mfma -S "$FIXTURE" -o "$WORK/fma.s" 2>"$WORK/cc2.log" \
    || { echo "FAIL: cannot emit assembly" >&2; cat "$WORK/cc2.log" >&2; exit 1; }

fail=0
# body <name>: the instructions between the label and the next `.size`.
body() {
    awk -v F="$1:" '$0==F{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$WORK/fma.s"
}
count() { printf '%s\n' "$1" | grep -cE '^[[:space:]]+[a-z]' || true; }

check() {   # check <name> <budget> <forbid-frame:0|1>
    local name=$1 budget=$2 no_frame=$3 text n
    text=$(body "$name")
    if [ -z "$text" ]; then
        echo "FAIL: no emitted body for '$name'" >&2
        fail=1; return
    fi
    n=$(count "$text")
    if [ "$n" -gt "$budget" ]; then
        echo "FAIL: $name costs $n instructions (budget $budget; pre-fix 28)." >&2
        printf '%s\n' "$text" | sed 's/^/        /' >&2
        fail=1
    fi
    if grep -qE 'andq[[:space:]]+\$-32' <<<"$text"; then
        echo "FAIL: $name still realigns a 32-byte frame home:" >&2
        printf '%s\n' "$text" | sed 's/^/        /' >&2
        fail=1
    fi
    if [ "$no_frame" = 1 ] \
        && grep -qE '^[[:space:]]+(subq|pushq)[[:space:]]' <<<"$text"; then
        echo "FAIL: $name allocates a frame; the value should stay in %ymm0-%ymm15:" >&2
        printf '%s\n' "$text" | sed 's/^/        /' >&2
        fail=1
    fi
    if [ "$fail" = 0 ]; then
        echo "  ok  $name costs $n instruction(s) (budget $budget)"
    fi
}

check fma_ps       3 1
check fma_pd       3 1
check fma_nested   6 0
check fma_chain    8 0
# Memory-operand shapes: the direct load's scratch home must be gone, so the
# moved address folds into the FMA's memory operand (or, for `fma_load_c`, into
# the single staging reload).  Pre-fix these cost 8-9 instructions plus
# `subq $72` and `andq $-32`.
check fma_load_a   3 1
check fma_load_b   3 1
check fma_load_c   4 1
check fma_load_pd  3 1
check load_only    3 1

if [ "$fail" != 0 ]; then
    if [ "$EXPECT_DEFECT" = 1 ]; then
        echo "PASS (expected defect): a register-home round trip was detected"
        exit 0
    fi
    exit 1
fi
if [ "$EXPECT_DEFECT" = 1 ]; then
    echo "FAIL: --expect-defect was requested but every shape is register-homed" >&2
    exit 1
fi

# --- 4. reference (optional) ----------------------------------------------
if [ -n "$REFERENCE" ] && command -v "$REFERENCE" >/dev/null 2>&1; then
    "$REFERENCE" -O2 -w -mavx2 -mfma -S "$FIXTURE" -o "$WORK/ref.s" 2>/dev/null || true
    if [ -s "$WORK/ref.s" ]; then
        for name in fma_ps fma_pd fma_nested fma_chain \
                    fma_load_a fma_load_b fma_load_c fma_load_pd load_only; do
            r=$(count "$(awk -v F="$name:" '$0==F{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$WORK/ref.s")")
            echo "  ref $REFERENCE: $name $r instruction(s)"
        done
    fi
fi

echo "PASS: every 256-bit FMA3 shape is register-homed within its budget"
