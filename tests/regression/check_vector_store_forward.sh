#!/usr/bin/env bash
# The pointer-form store builtins take their SOURCE as an address, so a
# computed vector source is materialised into a frame temporary and read
# straight back:
#
#   vmovups  %ymm4, 8(%rsp)     ; VecStore*  <- write the temporary
#   vmovdqu  8(%rsp), %ymm0     ; Storeu256  <- read it back
#   vmovdqu  %ymm0, (%rdi)      ; the store the source asked for
#
# `vector_store_forward` deletes both accesses when the temporary is written
# once by a whole-vector `VecStore*` and read once, by the store that follows
# it.  This gate pins the measured instruction counts, proves the counts go
# back up when the pass is disabled (so the budget is not satisfied by a
# different optimization), and executes every shape against a reference
# compiler so a wrong store is a failure and not a smaller number.
#
# Usage: check_vector_store_forward.sh [--lccc PATH] [--reference CC]
set -uo pipefail

LCCC="${LCCC:-./target/fastbuild/lccc}"
REFERENCE="${REFERENCE:-gcc-16}"
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        *) echo "check_vector_store_forward: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FIXTURE=tests/simd/vector_store_forward.c

if [ ! -x "$LCCC" ]; then
    echo "FAIL: no compiler at '$LCCC'" >&2
    exit 1
fi
if ! command -v "$REFERENCE" >/dev/null 2>&1; then
    echo "check_vector_store_forward: reference '$REFERENCE' unavailable; skipping the count comparison" >&2
    REFERENCE=""
fi

echo "check_vector_store_forward: whole-vector temporary stores are forwarded"

# --- 1. values -------------------------------------------------------------
# The fixture writes through both the forwarding shape and a dependent chain,
# then compares every byte against its own scalar model, and finally against
# the reference compiler's execution of the same file.
if ! "$LCCC" -O2 -w -mavx2 "$FIXTURE" -lm -o "$WORK/run" 2>"$WORK/cc.log"; then
    echo "FAIL: cannot build the value probe" >&2
    cat "$WORK/cc.log" >&2
    exit 1
fi
if "$WORK/run" >"$WORK/out.txt" 2>&1; then
    echo "  ok  $(cat "$WORK/out.txt")"
else
    echo "FAIL: the forwarded store disagrees with the scalar model" >&2
    cat "$WORK/out.txt" >&2
    exit 1
fi
# The same fixture under `-march=native`: the vectorizer's ISA selection and
# its indexed-load shapes change with the host feature set, and a load fold that
# mishandles a register offset is invisible at -mavx2 on a host whose native set
# takes a different path.
if ! "$LCCC" -O2 -w -march=native "$FIXTURE" -lm -o "$WORK/run-native" 2>"$WORK/cc-native.log"; then
    echo "FAIL: cannot build the -march=native value probe" >&2
    cat "$WORK/cc-native.log" >&2
    exit 1
fi
if "$WORK/run-native" >"$WORK/out-native.txt" 2>&1; then
    echo "  ok  -march=native $(cat "$WORK/out-native.txt")"
else
    echo "FAIL: the -march=native build disagrees with the scalar model" >&2
    cat "$WORK/out-native.txt" >&2
    exit 1
fi

if [ -n "$REFERENCE" ]; then
    "$REFERENCE" -O2 -w -mavx2 "$FIXTURE" -lm -o "$WORK/ref" 2>/dev/null \
        || { echo "FAIL: cannot build the value probe with $REFERENCE" >&2; exit 1; }
    "$WORK/ref" >"$WORK/ref.txt" 2>&1 || {
        echo "FAIL: $REFERENCE's build of the fixture failed its own model" >&2
        cat "$WORK/ref.txt" >&2
        exit 1
    }
    if ! diff -q "$WORK/out.txt" "$WORK/ref.txt" >/dev/null; then
        echo "FAIL: lccc and $REFERENCE disagree on the forwarded stores" >&2
        diff "$WORK/out.txt" "$WORK/ref.txt" >&2
        exit 1
    fi
    echo "  ok  every shape agrees with $REFERENCE"
fi

# --- 1b. the load fold's offset classification -----------------------------
# `try_elide_vec_load` folds a `VecLoad` into its consumer's memory operand.
# The offset slot means different things in the two IR layouts (an index
# register in the vectorizer's triple, a byte offset in the lowering's pair),
# and reading a REGISTER offset as "zero" folds every iteration at the base
# address.  The SLP object-identity corpus is the calibrated detector: its
# `word_map`/`f32_map` kernels compare a vectorized loop against a volatile
# scalar model, and they are wrong (values, not counts) when the fold
# misclassifies the offset.  The control is the fold itself:
# `CCC_NO_VLFOLD=1` keeps the staged load, which is always correct.
CORPUS=tests/simd/slp_object_identity.c
if [ -f "$CORPUS" ]; then
    "$LCCC" -O2 -w -march=native "$CORPUS" -o "$WORK/slp-on" 2>/dev/null \
        || { echo "FAIL: cannot build the SLP corpus" >&2; exit 1; }
    CCC_NO_VLFOLD=1 "$LCCC" -O2 -w -march=native "$CORPUS" -o "$WORK/slp-off" 2>/dev/null \
        || { echo "FAIL: cannot build the SLP corpus counterfactual" >&2; exit 1; }
    if ! "$WORK/slp-on" >"$WORK/slp-on.txt" 2>&1; then
        echo "FAIL: the SLP corpus disagrees with its scalar model with the load" >&2
        echo "      fold enabled, which is the misclassified-offset miscompile:" >&2
        grep -E 'FAIL' "$WORK/slp-on.txt" | sed -n '1,5p' >&2
        exit 1
    fi
    if ! "$WORK/slp-off" >"$WORK/slp-off.txt" 2>&1; then
        echo "FAIL: the SLP corpus fails with the fold DISABLED too; that is not" >&2
        echo "      a fold problem and this gate cannot isolate it:" >&2
        grep -E 'FAIL' "$WORK/slp-off.txt" | sed -n '1,5p' >&2
        exit 1
    fi
    echo "  ok  SLP indexed-load folding matches the scalar model (fold on and off)"
fi

# --- 2. counts, with an isolated disable control ---------------------------
# A temporary that is stored and read straight back has two ways out. The IR
# pass (`vecstorefwd`) deletes both accesses and the slot. Without it the
# slot survives into the assembly, and the peephole's store forwarding serves
# the reload from the register that wrote the slot: at 16 bytes always, at 32
# bytes since a 32-byte store has a mapping. For one reader either stage alone
# reaches the budget, so the control that has to cost something is BOTH off;
# for two readers only the IR pass has the shape, and that row is held to it.
body() {   # body <asm> <name>: instructions from the label to `.size`.
    awk -v F="$2:" '$0==F{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$1" \
        | grep -cE '^[[:space:]]+[a-z]'
}

"$LCCC" -O2 -w -mavx2 -S "$FIXTURE" -o "$WORK/on.s" 2>/dev/null \
    || { echo "FAIL: cannot emit assembly" >&2; exit 1; }
CCC_DISABLE_PASSES=vecstorefwd "$LCCC" -O2 -w -mavx2 -S "$FIXTURE" -o "$WORK/off.s" 2>/dev/null \
    || { echo "FAIL: cannot emit the counterfactual assembly" >&2; exit 1; }
# `store_fwd` is the forwarder's turn in phase 2 and `phase2b` its second one.
CCC_DISABLE_PASSES=vecstorefwd CCC_PEEPHOLE_SKIP=store_fwd,phase2b \
    "$LCCC" -O2 -w -mavx2 -S "$FIXTURE" -o "$WORK/none.s" 2>/dev/null \
    || { echo "FAIL: cannot emit the assembly without store forwarding" >&2; exit 1; }

fail=0
# check_budget <name> <budget>: the body must fit the budget and be non-empty.
# Used where no forwarding stage is a distinct counterfactual any more.
check_budget() {
    local name=$1 budget=$2 on
    on=$(body "$WORK/on.s" "$name")
    if [ "$on" -eq 0 ]; then
        echo "FAIL: no emitted body for '$name'" >&2
        fail=1; return
    fi
    if [ "$on" -gt "$budget" ]; then
        echo "FAIL: $name costs $on instructions (budget $budget)" >&2
        fail=1; return
    fi
    echo "  ok  $name costs $on instruction(s) (budget $budget; IR pass off" \
        "$(body "$WORK/off.s" "$name"), no forwarding $(body "$WORK/none.s" "$name"))"
}

# check <name> <budget> [ir]: the forwarded body must fit the budget, and with
# no forwarding stage at all it must be strictly worse. With `ir` the IR pass
# alone is the counterfactual: disabling just it must cost something too.
check() {
    local name=$1 budget=$2 held=${3:-} on off none bad=0
    on=$(body "$WORK/on.s" "$name")
    off=$(body "$WORK/off.s" "$name")
    none=$(body "$WORK/none.s" "$name")
    if [ "$on" -eq 0 ] || [ "$off" -eq 0 ] || [ "$none" -eq 0 ]; then
        echo "FAIL: no emitted body for '$name'" >&2
        fail=1; return
    fi
    if [ "$none" -le "$on" ]; then
        echo "FAIL: $name is $on instructions with store forwarding and $none without" >&2
        echo "      either stage; disabling both must cost something or this gate" >&2
        echo "      proves nothing" >&2
        bad=1
    fi
    if [ "$held" = ir ] && [ "$off" -le "$on" ]; then
        echo "FAIL: $name is $on instructions with the IR pass and $off without;" >&2
        echo "      only the IR pass has this shape, so disabling it must cost something" >&2
        bad=1
    fi
    if [ "$on" -gt "$budget" ]; then
        echo "FAIL: $name costs $on instructions (budget $budget, no forwarding $none)" >&2
        bad=1
    fi
    if [ "$bad" = 0 ]; then
        echo "  ok  $name costs $on instruction(s) (budget $budget; IR pass off $off," \
            "no forwarding $none)"
    else
        fail=1
    fi
}

# Measured, -O2 -mavx2: `on / IR pass off / no forwarding (GCC 16.2)`:
#   float_256   loop store of a computed 256-bit value   14 / 14 / 18 (13)
#   int_128     one 128-bit integer store                 3 /  3 /  7 (3)
#   ps_256      single-shot 256-bit float store           4 /  4 /  8 (3, GCC
#                                                        omits vzeroupper)
#   chain_256   loop-invariant splats + forwarded store 22 / 22 / 22 (17)
#   two_stores  one value read by two whole-vector stores 4 /  8 /  9 (4)
check float_256 14
check int_128 3
check ps_256 4
# chain_256 is budget-only. Its 256-bit result has a YMM register home (the
# straight-line 256-bit allocator in regalloc.rs), so the frame temporary that
# either stage would forward does not exist: 22 in every configuration.
check_budget chain_256 22

# A value read by TWO stores is the multi-reader case: every reader is an
# address-form store of the same width, so the IR pass forwards the register
# value to both and deletes the staging store and its frame slot.  Before the
# generalized proof this shape kept a full stack round trip (9 instructions
# with the pass, 9 without); it is 4 now, which is GCC's count exactly. The
# peephole serves the first reader and leaves the slot for the second.
check two_stores 4 ir

if [ "$fail" != 0 ]; then
    echo "FAIL: whole-vector temporary stores are not forwarded" >&2
    exit 1
fi
echo "PASS: whole-vector temporary stores are forwarded into the destination store"
