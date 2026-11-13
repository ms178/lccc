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
        grep -E 'FAIL' "$WORK/slp-on.txt" | head -5 >&2
        exit 1
    fi
    if ! "$WORK/slp-off" >"$WORK/slp-off.txt" 2>&1; then
        echo "FAIL: the SLP corpus fails with the fold DISABLED too; that is not" >&2
        echo "      a fold problem and this gate cannot isolate it:" >&2
        grep -E 'FAIL' "$WORK/slp-off.txt" | head -5 >&2
        exit 1
    fi
    echo "  ok  SLP indexed-load folding matches the scalar model (fold on and off)"
fi

# --- 2. counts, with an isolated disable control ---------------------------
body() {   # body <asm> <name>: instructions from the label to `.size`.
    awk -v F="$2:" '$0==F{on=1;next} on&&/^[[:space:]]*\.size/{exit} on' "$1" \
        | grep -cE '^[[:space:]]+[a-z]'
}

"$LCCC" -O2 -w -mavx2 -S "$FIXTURE" -o "$WORK/on.s" 2>/dev/null \
    || { echo "FAIL: cannot emit assembly" >&2; exit 1; }
CCC_DISABLE_PASSES=vecstorefwd "$LCCC" -O2 -w -mavx2 -S "$FIXTURE" -o "$WORK/off.s" 2>/dev/null \
    || { echo "FAIL: cannot emit the counterfactual assembly" >&2; exit 1; }

fail=0
# check <name> <budget>: the forwarded body must fit the budget, the disabled
# pass must be strictly worse, and no staging reload may survive.
check() {
    local name=$1 budget=$2 on off
    on=$(body "$WORK/on.s" "$name")
    off=$(body "$WORK/off.s" "$name")
    if [ "$on" -eq 0 ] || [ "$off" -eq 0 ]; then
        echo "FAIL: no emitted body for '$name'" >&2
        fail=1; return
    fi
    if [ "$off" -le "$on" ]; then
        echo "FAIL: $name is $on instructions with the pass and $off without;" >&2
        echo "      disabling it must cost something or this gate proves nothing" >&2
        fail=1
    fi
    if [ "$on" -gt "$budget" ]; then
        echo "FAIL: $name costs $on instructions (budget $budget, pre-fix $off)" >&2
        printf '%s\n' "$text" | sed 's/^/        /' >&2
        fail=1
    fi
    if [ "$fail" = 0 ]; then
        echo "  ok  $name costs $on instruction(s) (budget $budget, pre-fix $off)"
    fi
}

# Measured, -O2 -mavx2, `on / off (GCC 16.2)`:
#   float_256   loop store of a computed 256-bit value   14 / 18 (13)
#   int_128     one 128-bit integer store                 3 /  4 (3)
#   ps_256      single-shot 256-bit float store           4 /  8 (3, GCC
#                                                        omits vzeroupper)
#   chain_256   dependent stores, final one forwarded    23 / 25 (17)
check float_256 14
check int_128 3
check ps_256 4
check chain_256 23

# The control: this value is read by TWO stores, so its temporary is not
# single-reader and the rewrite must leave it exactly as it was.  A count
# difference here means the pass is not proving its premises.
control() {
    local name=$1 on off
    on=$(body "$WORK/on.s" "$name")
    off=$(body "$WORK/off.s" "$name")
    if [ "$on" -eq 0 ] || [ "$off" -eq 0 ]; then
        echo "FAIL: no emitted body for '$name'" >&2
        fail=1; return
    fi
    if [ "$on" -ne "$off" ]; then
        echo "FAIL: $name changed ($on with the pass, $off without) although its" >&2
        echo "      temporary has two readers; the rewrite must refuse it" >&2
        fail=1
        return
    fi
    echo "  ok  $name is untouched (two readers of the temporary)"
}
control two_stores

if [ "$fail" != 0 ]; then
    echo "FAIL: whole-vector temporary stores are not forwarded" >&2
    exit 1
fi
echo "PASS: whole-vector temporary stores are forwarded into the destination store"
