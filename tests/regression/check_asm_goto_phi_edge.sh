#!/usr/bin/env bash
# Asm-goto edges must survive i686 trivial-block deletion.
#
# Reproduced: -O0 -m32 deleted the PHI trampoline `.LBB5: jmp join` while the
# inline-asm `jnz .LBB5` still named it. GAS resolved the missing symbol to
# address 0 and the process segfaulted. x86-64 did not delete the label.
# The expected hash was executed by GCC 16.2.0, Clang 23.1.0, ICX 2024.0.0
# and ICC 2021.10.0 at -O2 and -Os.
#
# HERMETICITY (2026-11-08): the fixture used to include <stdint.h>/<stdio.h>.
# Those pull in glibc's multiarch bits/ tree, so on a host without
# libc6-dev-i386 EVERY -m32 leg died in the preprocessor and the gate was red
# for an environmental reason that has nothing to do with asm goto.  The C
# fixture is now header-free; the -S label invariant below is asserted on
# every -m32 leg regardless of the host, and the link+run leg is gated on a
# real 32-bit C-runtime capability probe (loud SKIP-RUN, never a false PASS).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
src=$root/tests/regression/asm_goto_phi_edge.c
tmp=${TMPDIR:-/tmp}/lccc-asm-goto-phi.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
expect=fd6c281bebb77835

# Pure text predicate; --self-test injects broken assembly into it.
asm_goto_label_present() {
    local asm=$1 target
    target=$(printf '%s\n' "$asm" |
        sed -n 's/.*jnz[[:space:]]\{1,\}\(\.LBB[0-9][0-9]*\).*/\1/p; T; q')
    [[ -n $target ]] || return 1
    grep -c "^${target}:" <<<"$asm" >/dev/null
}

# A real link probe, not a header guess: the 32-bit CRT/libc/libgcc must all
# be present for the run leg to mean anything.  Header-free on purpose.
m32_link_available() {
    printf 'int main(void){return 0;}\n' >"$tmp/probe.c"
    "$CCC" -m32 -x c "$tmp/probe.c" -o "$tmp/probe" >/dev/null 2>&1
}

if [[ ${1:-} == --self-test ]]; then
    asm_goto_label_present $'jnz .LBB5\n.LBB5:\n jmp join' ||
        { echo 'self-test: valid label rejected'; exit 1; }
    ! asm_goto_label_present $'jnz .LBB5\n.LBB6:\n jmp join' ||
        { echo 'self-test: dangling asm-goto label accepted'; exit 1; }
    ! asm_goto_label_present $'nop\nret\n' ||
        { echo 'self-test: assembly without an asm-goto accepted'; exit 1; }
    echo 'PASS asm-goto-phi-edge self-test (3 synthetic shape mutations rejected)'
    exit 0
fi

run_one() {
    local bits=$1 opt=$2
    # 1. The regression is an ASSEMBLER-SHAPE invariant: always asserted,
    #    with no dependency on a linker, a C runtime or an emulator.
    "$CCC" -m"$bits" -O"$opt" -Wall -Wextra -Werror -S "$src" -o "$tmp/t.s"
    local asm
    asm=$(cat "$tmp/t.s")
    if [[ $bits == 32 && $opt == 0 ]]; then
        if ! asm_goto_label_present "$asm"; then
            echo "FAIL -m$bits -O$opt: asm-goto target label was deleted"
            return 1
        fi
    fi
    # 2. Full link+run.  A 32-bit C runtime is a host capability, so probe it
    #    explicitly instead of letting a missing bits/*.h turn the gate red.
    if [[ $bits == 32 ]] && ! m32_link_available; then
        echo "SKIP-RUN: -m$bits -O$opt no 32-bit C runtime (-S label gate passed)"
        return
    fi
    "$CCC" -m"$bits" -O"$opt" -Wall -Wextra -Werror "$src" -o "$tmp/t"
    local interp
    interp=$(readelf -l "$tmp/t" | sed -n 's/.*Requesting program interpreter: \([^]]*\)\].*/\1/p')
    if [[ -n $interp && ! -e $interp ]]; then
        echo "SKIP-RUN: -m$bits -O$opt requires missing $interp"
        return
    fi
    local got
    got=$("$tmp/t")
    if [[ $got != "$expect" ]]; then
        echo "FAIL -m$bits -O$opt got=$got want=$expect"
        return 1
    fi
    echo "PASS -m$bits -O$opt $got"
}

bash "$0" --self-test
run_one 64 0
run_one 64 2
run_one 32 0
run_one 32 2
echo "PASS asm-goto PHI edge"
