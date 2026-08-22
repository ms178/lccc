#!/usr/bin/env bash
# Verify the AArch64 SSA-level conditional-increment selector and its A/B gate.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cc=${CCC_ARM:-$repo/target/fastbuild/lccc-arm}
src=$repo/tests/regression/arm_csinc_select.c
inc=${GCC_INC:-$(gcc -print-file-name=include)}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

"$cc" -O2 -I"$inc" -S "$src" -o "$tmp/enabled.s"
CCC_NO_CSINC_FOLD=1 "$cc" -O2 -I"$inc" -S "$src" -o "$tmp/disabled.s"

function_body() {
    local function=$1 file=$2
    awk -v wanted="$function" '
        $0 == wanted ":" { active=1 }
        active { print }
        active && $0 == ".size " wanted ", .-" wanted { exit }
    ' "$file"
}

positive=(
    inc_if_true_u32
    inc_if_false_u32
    inc_if_true_u64
    inc_if_eq_u32
    inc_if_ne_u32
    inc_if_slt_i32
    inc_if_sle_i32
    inc_if_sgt_i32
    inc_if_sge_i32
    inc_if_ult_u32
    inc_if_ule_u32
    inc_if_ugt_u32
    inc_if_uge_u32
    inc_loaded_condition
)
for function in "${positive[@]}"; do
    body=$(function_body "$function" "$tmp/enabled.s")
    grep -qE '^[[:space:]]+csinc[[:space:]]' <<<"$body" || {
        echo "FAIL: $function did not select csinc" >&2
        printf '%s\n' "$body" >&2
        exit 1
    }
    disabled=$(function_body "$function" "$tmp/disabled.s")
    if grep -qE '^[[:space:]]+csinc[[:space:]]' <<<"$disabled"; then
        echo "FAIL: CCC_NO_CSINC_FOLD did not disable $function" >&2
        exit 1
    fi
done

# CSINC increments when its own condition is false. Verify every signed and
# unsigned comparison inversion explicitly rather than merely checking that
# some CSINC was emitted.
declare -A expected_condition=(
    [inc_if_true_u32]=eq
    [inc_if_false_u32]=ne
    [inc_if_true_u64]=eq
    [inc_if_eq_u32]=ne
    [inc_if_ne_u32]=eq
    [inc_if_slt_i32]=ge
    [inc_if_sle_i32]=gt
    [inc_if_sgt_i32]=le
    [inc_if_sge_i32]=lt
    [inc_if_ult_u32]=hs
    [inc_if_ule_u32]=hi
    [inc_if_ugt_u32]=ls
    [inc_if_uge_u32]=lo
    [inc_loaded_condition]=eq
)
for function in "${!expected_condition[@]}"; do
    body=$(function_body "$function" "$tmp/enabled.s")
    condition=${expected_condition[$function]}
    grep -qE "^[[:space:]]+csinc[[:space:]].*,[[:space:]]*$condition$" <<<"$body" || {
        echo "FAIL: $function did not use expected CSINC condition $condition" >&2
        printf '%s\n' "$body" >&2
        exit 1
    }
done

for function in do_not_fold_delta_two do_not_fold_extra_use; do
    body=$(function_body "$function" "$tmp/enabled.s")
    if grep -qE '^[[:space:]]+csinc[[:space:]]' <<<"$body"; then
        echo "FAIL: ineligible $function selected csinc" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
done

# Ensure every selected instruction is accepted by the independent GNU
# assembler. This catches width/condition/register encoding mistakes.
aarch64-linux-gnu-gcc -c "$tmp/enabled.s" -o "$tmp/enabled.o"

echo "PASS: AArch64 conditional-increment selection and gate"
