#!/usr/bin/env bash
# Gate: the packed integer/bitwise map families must stay register-homed.
#
# The defect this gate pins (measured on the fastbuild tree of 2026-11-12):
# every pointer-form packed map op takes one ADDRESS per operand and writes
# its result through a `dest_ptr`, so a register-resident operand is spilled
# into a frame slot first:
#
#     __m128i f(__m128i a, __m128i b){ return _mm_add_epi8(a,b); }
#         subq $88, %rsp
#         movdqu %xmm1, 32(%rsp)          ; operand home
#         vpaddb 32(%rsp), %xmm0, %xmm0   ; the arithmetic, on memory
#         addq $88, %rsp
#         ret
#
# GCC 16.2 emits `vpaddb %xmm1, %xmm0, %xmm0; ret` (2).  The register twin
# (`VecAddI8x16`) does the same arithmetic with no frame, so the routing in
# `try_native_packed_map` is what these budgets measure: 5 -> 2 at 128 bits,
# 9 -> 2 at 256 bits, for add/sub/and/or/xor/mul/min/max, the immediate
# shifts, the eq/gt compares and the float bitwise forms.
#
# Three independent assertions, because they fail independently:
#
#   1. VALUES.  The fixture EXECUTES every family for every lane width (i8/
#      i16/i32/i64, u8, s16) at both register widths against a scalar
#      reference computed in the file, lane by lane through the byte image.
#      A lane map is written by hand per builtin, and `Paddb` and `Paddq`
#      both encode in ONE instruction -- an instruction count cannot tell a
#      byte add from a qword add, so this half is what catches a wrong row
#      in the twin table.  The same file is also built by the reference
#      compiler (`--reference`, gcc-16 in CI): if the scalar reference model
#      in the fixture were wrong, the two compilers would disagree with it
#      in the same direction and the differential would hide it.
#   2. HOMING.  Each counted shape stays inside its instruction budget AND —
#      for the register shapes — emits no frame at all (`subq`/`pushq`) and
#      touches no stack slot (`(%rsp)`).  Every pointer-form lowering needs
#      a frame (the homes are dynamically realigned for the 32-byte cases),
#      so the absence of a frame is a positive proof that the register path
#      ran, not merely that the count is small.
#   3. FALLBACK INTEGRITY.  The twin table deliberately refuses shapes whose
#      meaning the register twin would change (shift count 0, a count above
#      the lane width) and volatile/foreign-space operands.  Those keep the
#      pointer path and are checked twice: their VALUES are in the fixture,
#      and the detector self-test below proves a deliberately-kept pointer
#      shape still shows the frame idiom that assertion 2 forbids.
#
# Non-vacuity: `--expect-defect` subtracts one instruction from every budget
# and requires the gate to fail, which is what makes the counting harness
# itself testable.  `--m32` runs the same value probe for i686, where the
# backend does not implement the value form of every twin and the pointer
# path is therefore still the shipping lowering; there only the VALUES are
# asserted.
#
# Usage: check_x86_int_map_registers.sh [--lccc PATH] [--reference CC]
#                                      [--expect-defect]
set -euo pipefail

LCCC=""; REFERENCE=""; EXPECT_DEFECT=0
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --expect-defect) EXPECT_DEFECT=1; shift ;;
        -h|--help) sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "check_x86_int_map_registers: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CC="${LCCC:-$ROOT/target/fastbuild/lccc}"
FIXTURE="$ROOT/tests/simd/x86_int_map_registers.c"
[ -x "$CC" ] || { echo "FAIL: lccc binary '$CC' is not executable" >&2; exit 1; }
[ -r "$FIXTURE" ] || { echo "FAIL: fixture '$FIXTURE' is missing" >&2; exit 1; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
echo "check_x86_int_map_registers: packed integer/bitwise register homing"

fail=0
FLAGS=(-O2 -w -mavx2 -mfma)

# --- 1. values -------------------------------------------------------------
if ! "$CC" "${FLAGS[@]}" "$FIXTURE" -o "$WORK/run" 2>"$WORK/cc.log"; then
    echo "FAIL: cannot build the value probe" >&2
    cat "$WORK/cc.log" >&2
    exit 1
fi
if "$WORK/run" >"$WORK/out.txt" 2>&1; then
    echo "  ok  $(cat "$WORK/out.txt")"
else
    echo "FAIL: a packed map family disagrees with the scalar reference" >&2
    cat "$WORK/out.txt" >&2
    exit 1
fi

# The rewrite is a LOWERING decision, so it is exercised at every -O level;
# the optimizers only decide whether the result home survives.  A wrong lane
# width (the failure mode an instruction count cannot see) is a lowering bug
# and must be caught with the optimizers out of the picture too.
for opt in -O0 -O1 -O3; do
    if ! "$CC" "$opt" -w -mavx2 -mfma "$FIXTURE" -o "$WORK/run$opt" 2>"$WORK/cc$opt.log"; then
        echo "FAIL: cannot build the value probe at $opt" >&2
        cat "$WORK/cc$opt.log" >&2
        exit 1
    fi
    if "$WORK/run$opt" >"$WORK/out$opt.txt" 2>&1; then
        echo "  ok  $opt agrees with the scalar reference"
    else
        echo "FAIL: the map families disagree with the reference at $opt" >&2
        cat "$WORK/out$opt.txt" >&2
        exit 1
    fi
done

# --- 2. the reference model itself ----------------------------------------
if [ -n "$REFERENCE" ]; then
    if ! "$REFERENCE" "${FLAGS[@]}" "$FIXTURE" -o "$WORK/ref" 2>"$WORK/ref.log"; then
        echo "FAIL: the reference compiler cannot build the value probe" >&2
        cat "$WORK/ref.log" >&2
        exit 1
    fi
    if "$WORK/ref" >"$WORK/ref.txt" 2>&1; then
        echo "  ok  $REFERENCE agrees with the same scalar reference"
    else
        echo "FAIL: $REFERENCE disagrees with the fixture's scalar reference," >&2
        echo "      so the reference model itself is wrong:" >&2
        cat "$WORK/ref.txt" >&2
        exit 1
    fi
fi

# --- 3. detector self-test -------------------------------------------------
# A volatile operand legitimately keeps the pointer form (the routing refuses
# volatile objects), so its body MUST still allocate a frame.  That makes the
# "no frame" assertion below non-vacuous: it runs against text that really
# carries the shape it forbids.
cat > "$WORK/legacy.c" <<'EOF'
#include <emmintrin.h>
__m128i legacy_probe(volatile __m128i *p, __m128i b) {
    return _mm_add_epi8(*p, b);
}
EOF
"$CC" "${FLAGS[@]}" -S "$WORK/legacy.c" -o "$WORK/legacy.s" 2>"$WORK/legacy.log" \
    || { echo "FAIL: cannot emit the detector self-test" >&2; cat "$WORK/legacy.log" >&2; exit 1; }
legacy_body=$(awk '/^legacy_probe:/{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$WORK/legacy.s")
if grep -qE '^[[:space:]]+subq[[:space:]]' <<<"$legacy_body"; then
    echo "  ok  detector self-test: the pointer form still allocates a frame"
else
    echo "FAIL: the detector self-test emitted no frame, so the homing" >&2
    echo "      assertions below could not fail.  Body:" >&2
    printf '%s\n' "$legacy_body" | sed 's/^/        /' >&2
    exit 1
fi

# --- 4. homing -------------------------------------------------------------
"$CC" "${FLAGS[@]}" -S "$FIXTURE" -o "$WORK/map.s" 2>"$WORK/cc2.log" \
    || { echo "FAIL: cannot emit assembly" >&2; cat "$WORK/cc2.log" >&2; exit 1; }

# body <name>: the instructions between the label and the next `.size`.
body() {
    awk -v F="$1:" '$0==F{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$WORK/map.s"
}
count() { printf '%s\n' "$1" | grep -cE '^[[:space:]]+[a-z]' || true; }

# check <name> <budget> <stack-free:0|1>
check() {
    local name=$1 budget=$2 clean=$3 text n
    if [ "$EXPECT_DEFECT" = 1 ]; then
        budget=$((budget - 1))
    fi
    text=$(body "$name")
    if [ -z "$text" ]; then
        echo "FAIL: no emitted body for '$name'" >&2
        fail=1; return
    fi
    n=$(count "$text")
    if [ "$n" -gt "$budget" ]; then
        echo "FAIL: $name costs $n instructions (budget $budget; pointer form 5/9)." >&2
        printf '%s\n' "$text" | sed 's/^/        /' >&2
        fail=1
    fi
    if [ "$clean" = 1 ]; then
        if grep -qE '^[[:space:]]+(subq|pushq)[[:space:]]' <<<"$text"; then
            echo "FAIL: $name allocates a frame; the value should stay in %xmm/%ymm:" >&2
            printf '%s\n' "$text" | sed 's/^/        /' >&2
            fail=1
        fi
        if grep -qE '\(%rsp\)|\(%rbp\)' <<<"$text"; then
            echo "FAIL: $name still reads or writes a stack slot:" >&2
            printf '%s\n' "$text" | sed 's/^/        /' >&2
            fail=1
        fi
    fi
}

# Register-resident shapes: one packed instruction plus `ret`.
for name in map_add8_128 map_add8_256 map_and_128 map_and_256 \
            map_shift_128 map_shift_256 map_cmp_128 map_cmp_256 \
            map_mul16_128 map_mul32_256 map_min_128 map_max_256; do
    check "$name" 2 1
done
# Memory-operand shapes: the folded load, the operation, `ret`.  The other
# operand is a by-value vector parameter whose frame home belongs to the ABI
# (RH-04), which is why these are one above GCC instead of at parity.
check map_add8m_128 3 0
check map_add64m_256 3 0

# Inlined wrappers: a `target(...)`-attributed always-inline callee with two
# by-value 16/32-byte vector parameters.  The parameter bridge used to cost a
# 16-byte store/load pair per parameter, so one packed instruction came with
# a frame.  `forward_memcpy_filled_reads` forwards the reads to the caller's
# register home, so `map_wrap_add16_128` is `vpaddw %xmm1, %xmm0, %xmm0`.
#
# The two extrema wrappers get a budget of 3, not 2, for the same reason GCC
# needs `vpmaxuw`: there is no min/max-without-AVX-512 value form, so the
# header uses the exact saturating-difference identity (`vpsubusw` plus one
# `vpaddw`/`vpsubw`).  That is one instruction above GCC, and it is a property
# of the target ISA, not of the parameter bridge: without the bridge the same
# shapes cost 5 and 8.
WRAPPERS="map_wrap_add16_128:2 map_wrap_maxu16_128:3 map_wrap_minu16_256:3 map_wrap_and256_256:2"
for entry in $WRAPPERS; do
    check "${entry%%:*}" "${entry##*:}" 1
done

if [ "$fail" != 0 ]; then
    if [ "$EXPECT_DEFECT" = 1 ]; then
        echo "PASS (expected defect): the instruction budgets can fail"
        exit 0
    fi
    exit 1
fi
if [ "$EXPECT_DEFECT" = 1 ]; then
    echo "FAIL: --expect-defect expected an overrun and found none" >&2
    exit 1
fi
echo "  ok  every counted shape is register-homed (12 at parity, 2 memory forms, 4 wrappers)"

# --- 5. counterfactual -----------------------------------------------------
# `CCC_NO_NATIVE_PACKED_MAP` reverts the routing at the one place that makes
# the decisions, so this is the PRE-FIX lowering rather than a hand-edited
# budget.  It must blow the budgets and must NOT change a single value: the
# switch is a quality switch, and a gate that could only ever pass would not
# be a gate.
if [ -z "${CCC_NO_NATIVE_PACKED_MAP-}" ]; then
    CCC_NO_NATIVE_PACKED_MAP=1 "$CC" "${FLAGS[@]}" -S "$FIXTURE" -o "$WORK/off.s" \
        2>"$WORK/cc3.log" \
        || { echo "FAIL: cannot emit the counterfactual assembly" >&2
             cat "$WORK/cc3.log" >&2; exit 1; }
    for name in map_add8_128 map_add8_256 map_cmp_256; do
        off_body=$(awk -v F="$name:" '$0==F{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$WORK/off.s")
        off_n=$(count "$off_body")
        if [ "$off_n" -le 2 ]; then
            echo "FAIL: with CCC_NO_NATIVE_PACKED_MAP=1, $name still costs $off_n" >&2
            echo "      instructions -- the routing under test is not what the" >&2
            echo "      budgets measure, so this gate is vacuous." >&2
            exit 1
        fi
    done
    if ! CCC_NO_NATIVE_PACKED_MAP=1 "$CC" "${FLAGS[@]}" "$FIXTURE" -o "$WORK/run_off" 2>/dev/null \
        || ! "$WORK/run_off" >"$WORK/out_off.txt" 2>&1; then
        echo "FAIL: the pointer fallback changed a value" >&2
        cat "$WORK/out_off.txt" >&2 2>/dev/null || true
        exit 1
    fi
    echo "  ok  counterfactual: the pre-fix lowering costs 5/9 and computes the same values"
fi

# The wrapper shapes have their own counterfactual: switching the parameter
# forwarding off must bring the copies (and the frame) back, otherwise those
# budgets do not measure the transform they claim to measure.
if [ -z "${CCC_NO_AGG_MEMCPY_FWD-}" ]; then
    CCC_NO_AGG_MEMCPY_FWD=1 "$CC" "${FLAGS[@]}" -S "$FIXTURE" -o "$WORK/nofwd.s" \
        2>"$WORK/cc4.log" \
        || { echo "FAIL: cannot emit the counterfactual assembly" >&2
             cat "$WORK/cc4.log" >&2; exit 1; }
    for entry in $WRAPPERS; do
        name=${entry%%:*}
        budget=${entry##*:}
        off_body=$(awk -v F="$name:" '$0==F{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$WORK/nofwd.s")
        off_n=$(count "$off_body")
        if [ "$off_n" -le "$budget" ]; then
            echo "FAIL: with CCC_NO_AGG_MEMCPY_FWD=1, $name still costs $off_n" >&2
            echo "      instructions (budget $budget) -- the wrapper budgets are" >&2
            echo "      vacuous." >&2
            exit 1
        fi
    done
    if ! CCC_NO_AGG_MEMCPY_FWD=1 "$CC" "${FLAGS[@]}" "$FIXTURE" -o "$WORK/run_nofwd" 2>/dev/null \
        || ! "$WORK/run_nofwd" >"$WORK/out_nofwd.txt" 2>&1 \
        || ! grep -q "PASS" "$WORK/out_nofwd.txt"; then
        echo "FAIL: switching the parameter forwarding off changed a value" >&2
        cat "$WORK/out_nofwd.txt" >&2 2>/dev/null || true
        exit 1
    fi
    echo "  ok  counterfactual: without the parameter forwarding each wrapper pays a frame"
fi

# --- 5. i686: the pointer fallback must stay correct ----------------------
# The i686 backend implements the value form of only a subset of the twins
# (`emit_intrinsic` refuses the rest), so 32-bit keeps the pointer path.  The
# values still have to be right -- that path is what ships there.
if "$CC" -m32 -E "$FIXTURE" -o /dev/null 2>/dev/null; then
    if ! "$CC" -m32 "${FLAGS[@]}" "$FIXTURE" -o "$WORK/run32" 2>"$WORK/cc32.log"; then
        echo "FAIL: cannot build the i686 value probe" >&2
        cat "$WORK/cc32.log" >&2
        exit 1
    fi
    if "$WORK/run32" >"$WORK/out32.txt" 2>&1; then
        echo "  ok  i686 (pointer fallback): $(cat "$WORK/out32.txt")"
    else
        echo "FAIL: the i686 pointer fallback disagrees with the reference" >&2
        cat "$WORK/out32.txt" >&2
        exit 1
    fi
else
    echo "  skip i686 value probe (this compiler has no -m32 support)"
fi

echo "PASS: packed integer/bitwise map families are register-homed and exact"
