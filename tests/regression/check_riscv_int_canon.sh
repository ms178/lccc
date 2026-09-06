#!/usr/bin/env bash
# Differential runner for tests/regression/riscv_int_canon.c (rv64).
#
# lccc-riscv and riscv64 gcc compile the SAME source; both binaries run
# under qemu-riscv64 and must agree on stdout and exit code for every
# opt level.  Uses the built GNU as 2.47 when present (the differential
# suites hard-gate on the latest assembler); otherwise the cross
# toolchain's assembler.
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
lccc=${CCC_RISCV:-$repo/target/fastbuild/lccc-riscv}
xgcc=${CCC_RISCV_GCC:-riscv64-linux-gnu-gcc}
qemu=${CCC_QEMU:-qemu-riscv64}
as247=/home/user/.cache/gas-2.47-riscv64-linux-gnu/bin/as
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

command -v "$xgcc" >/dev/null || { echo "SKIP: $xgcc not available"; exit 0; }
command -v "$qemu" >/dev/null || { echo "SKIP: $qemu not available"; exit 0; }

src="$repo/tests/regression/riscv_int_canon.c"
fail=0
for opt in -O0 -O1 -O2; do
    # lccc side: assemble with gas 2.47 when provisioned, else cross as.
    "$lccc" $opt "$src" -S -o "$tmp/l.s" || { echo "FAIL(lccc compile $opt)"; fail=1; continue; }
    if [ -x "$as247" ]; then "$as247" -o "$tmp/l.o" "$tmp/l.s"; else "$xgcc" -c "$tmp/l.s" -o "$tmp/l.o"; fi
    "$xgcc" -static "$tmp/l.o" -o "$tmp/l" || { echo "FAIL(link $opt)"; fail=1; continue; }
    lout=$("$qemu" "$tmp/l" 2>&1); lec=$?
    # gcc side straight from source.
    "$xgcc" $opt -static "$src" -o "$tmp/g" || { echo "SKIP(gcc cannot build $opt)"; continue; }
    gout=$("$qemu" "$tmp/g" 2>&1); gec=$?
    if [ "$lout" = "$gout" ] && [ "$lec" = "$gec" ]; then
        echo "PASS $opt (exit $lec)"
    else
        echo "FAIL $opt: lccc(exit=$lec) vs gcc(exit=$gec)"
        diff <(printf '%s\n' "$gout") <(printf '%s\n' "$lout") | head -10
        fail=1
    fi
done
exit $fail
