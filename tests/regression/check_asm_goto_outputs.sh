#!/usr/bin/env bash
# Runtime and codegen regressions for GNU asm-goto outputs, including flag
# outputs on both fall-through and implicit label edges.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
src=$root/tests/regression/asm_goto_outputs.c
tmp=${TMPDIR:-/tmp}/lccc-asm-goto-outputs.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

run_one() {
    local bits=$1 opt=$2
    "$CCC" -m"$bits" -std=gnu11 -O"$opt" -Wall -Wextra -Werror "$src" -o "$tmp/test"
    local interp
    interp=$(readelf -l "$tmp/test" | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
    if [[ -n $interp && ! -e $interp ]]; then
        echo "SKIP-RUN: -m$bits -O$opt requires missing $interp"
        return
    fi
    local got
    got=$("$tmp/test")
    [[ $got == "PASS asm_goto_outputs" ]]
    echo "PASS -m$bits -O$opt $got"
}

run_one 64 0
run_one 64 2
run_one 32 0
run_one 32 2

# Guard the x86-64 -O2 win that motivates the direct-flags path: the C branch
# after the asm-goto label must not reload the materialized bool from the
# stack. Other assembly details are intentionally left unconstrained.
"$CCC" -m64 -O2 -S "$src" -o "$tmp/test.s"
func=$(sed -n '/^asm_cc_output_branch:/,/^\.size asm_cc_output_branch/p' "$tmp/test.s")
[[ -n $func ]]
grep -Eq '^[[:space:]]*j[a-z]+[[:space:]]' <<<"$func"
if grep -Eq 'movzbl[[:space:]]+[^,]*\(%rsp\)' <<<"$func"; then
    echo "FAIL: asm-goto flag-output branch reloads its stack bool" >&2
    exit 1
fi
echo "PASS x86-64 direct condition-code branches (no stack bool reload)"
