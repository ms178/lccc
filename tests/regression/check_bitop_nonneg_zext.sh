#!/usr/bin/env bash
# Verify the cross-backend bitop non-negative widening-cast transfer:
# Clz/Ctz/Popcount results on ≤32-bit types are in [0, 32], so the I32→I64
# signed widen skips the sign-extension on AArch64 (the W-register write
# already zeroed the upper half — sxtw dead) and on RISC-V (sext.w is the
# identity on the from-zero-upward count). The plain-integer control keeps
# its extension, and the emitted assembly must assemble with lccc itself.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
arm=${CCC_ARM:-$repo/target/fastbuild/lccc-arm}
riscv=${CCC_RISCV:-$repo/target/fastbuild/lccc-riscv}
src=$repo/tests/regression/bitop_nonneg_zext.c
inc=${GCC_INC:-$(gcc -print-file-name=include)}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

"$arm" -O2 -I"$inc" -S "$src" -o "$tmp/arm.s"
"$riscv" -O2 -I"$inc" -S "$src" -o "$tmp/riscv.s"

function_body() {
    local function=$1 file=$2
    awk -v wanted="$function" '
        $0 == wanted ":" { active=1 }
        active { print }
        active && $0 == ".size " wanted ", .-" wanted { exit }
    ' "$file"
}

bitops=(widen_clz widen_ctz widen_pop)

for function in "${bitops[@]}"; do
    body=$(function_body "$function" "$tmp/arm.s")
    if grep -qE '^[[:space:]]+sxtw[[:space:]]' <<<"$body"; then
        echo "FAIL: arm $function still sign-extends a non-negative bitop" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
    body=$(function_body "$function" "$tmp/riscv.s")
    if grep -qE '^[[:space:]]+sext\.w[[:space:]]' <<<"$body"; then
        echo "FAIL: riscv $function still sign-extends a non-negative bitop" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
done

# Control: a plain signed int widen must keep the extension on both targets.
body=$(function_body widen_plain "$tmp/arm.s")
grep -qE '^[[:space:]]+sxtw[[:space:]]' <<<"$body" || {
    echo "FAIL: arm widen_plain lost its sign-extension" >&2
    printf '%s\n' "$body" >&2
    exit 1
}
body=$(function_body widen_plain "$tmp/riscv.s")
grep -qE '^[[:space:]]+sext\.w[[:space:]]' <<<"$body" || {
    echo "FAIL: riscv widen_plain lost its sign-extension" >&2
    printf '%s\n' "$body" >&2
    exit 1
}

# The emitted assembly must be accepted by lccc's own assemblers — this
# catches any malformed mnemonic/register shape the grep above cannot.
"$arm" -c "$tmp/arm.s" -o "$tmp/arm.o"
"$riscv" -c "$tmp/riscv.s" -o "$tmp/riscv.o"

echo "PASS: bitop non-negative widening-cast transfer (AArch64 + RISC-V)"
