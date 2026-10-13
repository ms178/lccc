#!/usr/bin/env bash
# Asm-goto edges must survive i686 trivial-block deletion.
#
# Reproduced: -O0 -m32 deleted the PHI trampoline `.LBB5: jmp join` while the
# inline-asm `jnz .LBB5` still named it. GAS resolved the missing symbol to
# address 0 and the process segfaulted. x86-64 did not delete the label.
# The expected hash was executed by GCC 16.2.0, Clang 23.1.0, ICX 2024.0.0
# and ICC 2021.10.0 at -O2 and -Os.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
src=$root/tests/regression/asm_goto_phi_edge.c
tmp=${TMPDIR:-/tmp}/lccc-asm-goto-phi.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
expect=fd6c281bebb77835

run_one() {
    local bits=$1 opt=$2
    "$CCC" -m"$bits" -O"$opt" -Wall -Wextra -Werror "$src" -o "$tmp/t"
    "$CCC" -m"$bits" -O"$opt" -S "$src" -o "$tmp/t.s"
    local asm
    asm=$(cat "$tmp/t.s")
    if [[ $bits == 32 && $opt == 0 ]]; then
        local target
        target=$(printf '%s\n' "$asm" | sed -n 's/.*jnz[[:space:]]\{1,\}\(\.LBB[0-9][0-9]*\).*/\1/p; T; q')
        [[ -n $target ]]
        grep -c "^${target}:" <<<"$asm" >/dev/null
    fi
    local interp
    interp=$(readelf -l "$tmp/t" | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
    if [[ -n $interp && ! -e $interp ]]; then
        echo "SKIP-RUN: -m$bits -O$opt requires missing $interp"
        return
    fi
    local got
    got=$("$tmp/t")
    [[ $got == "$expect" ]]
    echo "PASS -m$bits -O$opt $got"
}

run_one 64 0
run_one 64 2
run_one 32 0
run_one 32 2
echo "PASS asm-goto PHI edge"
