#!/usr/bin/env bash
# Run-time witness of RISC-V code layout under linker relaxation
# (RV-AS-ALIGN, RV-LD-ALIGN, RV-AS-LABELDIFF, RV-AS-SECSYM, RV-AS-ULEB128).
#
# riscv_relax/relax_layout.c is freestanding (no libc), so the core legs need
# only lccc-riscv, qemu-riscv64 and ld.lld -- all present on the CI runner.
# Its exit status counts failed run-time checks: an over-aligned function's
# address, a jump table, a computed-goto table and hand-written `.word` /
# `.uleb128` / `. -` spans, all over code a relaxing linker shrinks.
#
#   lccc object + lccc-ld          (ALIGN runs trimmed by riscv/linker/align.rs)
#   lccc object + ld.lld           (relaxes call/tail; trims ALIGN runs)
#   lccc object + GNU ld           (when riscv64-linux-gnu-ld is installed)
#   GAS object  + lccc-ld          (when riscv64-linux-gnu-gcc is installed)
#
# Every leg runs at -O0 and -O2.  A missing optional tool is a SKIP line, never
# a PASS; a missing core tool skips the whole gate (as the other qemu gates do).
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
lccc=${CCC_RISCV:-$repo/target/fastbuild/lccc-riscv}
qemu=${CCC_QEMU:-qemu-riscv64}
lld=${CCC_LLD:-ld.lld}
bfd=${CCC_RISCV_LD:-riscv64-linux-gnu-ld}
xgcc=${CCC_RISCV_GCC:-riscv64-linux-gnu-gcc}
src=$repo/tests/regression/riscv_relax/relax_layout.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$lccc" ] || { echo "SKIP: $lccc not built"; exit 0; }
command -v "$qemu" >/dev/null || { echo "SKIP: $qemu not available"; exit 0; }

fail=0
run() { # name exe
    "$qemu" "$2"
    local rc=$?
    if [ "$rc" -eq 0 ]; then
        echo "PASS $1"
    elif [ "$rc" -gt 128 ]; then
        echo "FAIL $1: killed by signal $((rc - 128))"
        fail=1
    else
        echo "FAIL $1: $rc run-time check(s) failed"
        fail=1
    fi
}
for opt in -O0 -O2; do
    o=$tmp/l$opt.o
    if ! "$lccc" $opt -ffreestanding -fno-pic -c "$src" -o "$o"; then
        echo "FAIL lccc compile $opt"; fail=1; continue
    fi
    if "$lccc" -nostdlib -static -Wl,-e,_start "$o" -o "$tmp/a"; then
        run "lccc+lccc-ld $opt" "$tmp/a"
    else
        echo "FAIL lccc-ld link $opt"; fail=1
    fi
    if command -v "$lld" >/dev/null; then
        if "$lld" -static -e _start "$o" -o "$tmp/b"; then
            run "lccc+ld.lld $opt" "$tmp/b"
        else
            echo "FAIL ld.lld link $opt"; fail=1
        fi
    else
        echo "SKIP lccc+ld.lld $opt: $lld not available"
    fi
    if command -v "$bfd" >/dev/null; then
        if "$bfd" -static -e _start "$o" -o "$tmp/c"; then
            run "lccc+GNU-ld $opt" "$tmp/c"
        else
            echo "FAIL GNU ld link $opt"; fail=1
        fi
    else
        echo "SKIP lccc+GNU-ld $opt: $bfd not available"
    fi
    if command -v "$xgcc" >/dev/null; then
        g=$tmp/g$opt.o
        if "$xgcc" $opt -ffreestanding -fno-pic -c "$src" -o "$g" &&
            "$lccc" -nostdlib -static -Wl,-e,_start "$g" -o "$tmp/d"; then
            run "GAS+lccc-ld $opt" "$tmp/d"
        else
            echo "FAIL GAS object through lccc-ld $opt"; fail=1
        fi
    else
        echo "SKIP GAS+lccc-ld $opt: $xgcc not available"
    fi
done
exit $fail
