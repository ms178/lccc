#!/usr/bin/env bash
set -euo pipefail
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
source "$here/tls_runtime_oracle.sh"
CCC=${CCC:-$here/../../target/fastbuild/lccc}
GCC=${GCC:-gcc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$here/volatile_reduction.c
"$GCC" -O3 -fno-pic -no-pie "$src" -o "$work/reference"
for flags in '-O3 -fno-pic -no-pie' '-O3 -fPIE -pie' '-O3 -fPIC' '-O3 -g -fno-pic -no-pie' '-O2' '-Os' '-O3 -mgeneral-regs-only -fno-pic -no-pie' '-O3 -march=x86-64 -fno-pic -no-pie'; do
    CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1 "$CCC" $flags "$src" -o "$work/run"
    tls_compare_runtime "$work/run" "$work/reference" "$work/out" 30
done
for disabled in volatile_reduce unroll; do
    CCC_DISABLE_PASSES=$disabled "$CCC" -O3 -fno-pic -no-pie "$src" -o "$work/control"
    tls_compare_runtime "$work/control" "$work/reference" "$work/control-out" 30
done
# Check mechanism independently of constant-value output parity. Four main-loop
# accesses plus one scalar-tail access must survive; the disabled control has one.
"$CCC" -O3 -fno-pic -S "$src" -o "$work/on.s"
CCC_DISABLE_PASSES=volatile_reduce "$CCC" -O3 -fno-pic -S "$src" -o "$work/off.s"
for python in 'python3' 'python3 -O'; do
    $python "$here/check_volatile_reduction_asm.py" "$work/on.s" 5
    $python "$here/check_volatile_reduction_asm.py" "$work/off.s" 1
done
for policy in '-O2' '-Os' '-O3'; do
    if [[ $policy == -O3 ]]; then
        CCC_DISABLE_PASSES=unroll "$CCC" $policy -fno-pic -S "$src" -o "$work/policy.s"
    else
        "$CCC" $policy -fno-pic -S "$src" -o "$work/policy.s"
    fi
    python3 "$here/check_volatile_reduction_asm.py" "$work/policy.s" 1
done
# Preserve main's lane-zero compatibility control while hardening fixed-four.
CCC_VOLATILE_REDUCE_LANES=0 "$CCC" -O3 -fno-pic -S "$src" -o "$work/lane-zero.s"
python3 "$here/check_volatile_reduction_asm.py" "$work/lane-zero.s" 1
CCC_VOLATILE_REDUCE_LANES=0 "$CCC" -O3 -fno-pic -no-pie "$src" -o "$work/lane-zero"
tls_compare_runtime "$work/lane-zero" "$work/reference" "$work/lane-zero-out" 30
python3 "$here/test_volatile_reduction_asm.py"
python3 "$here/../../scripts/test_measure_volatile_reductions.py"
python3 -O "$here/test_volatile_reduction_asm.py"
# Inspect the real pipeline checkpoint, not just runtime parity, for both
# widths, bitwise operations and both Local-Exec and shared-object TLS models.
for model in -fno-pic -fPIC; do
    CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=add32,xor64,tls_product,live_out \
        "$CCC" -O3 "$model" -S "$src" -o "$work/ir.s" 2>"$work/ir.log"
    python3 - "$here" "$work/ir.log" "$model" <<'PY_CHECK'
import pathlib, sys
sys.path.insert(0, sys.argv[1])
from check_volatile_reduction_asm import ir_accesses
text = pathlib.Path(sys.argv[2]).read_text()
for symbol in ("add32", "xor64", "tls_product", "live_out"):
    count = ir_accesses(text, symbol)
    expected = 1 if symbol == "tls_product" and sys.argv[3] == "-fPIC" else 5
    if count != expected:
        raise SystemExit(f"{symbol}: expected {expected} volatile loads, found {count}")
PY_CHECK
done
# Refusal cases originate from valid C/frontend SSA, not malformed fixtures.
CCC_DUMP_EACH_PASS=1 "$CCC" -O3 -S "$here/volatile_reduction_refusals.c" \
    -o "$work/refusals.s" 2>"$work/refusals.log"
python3 - "$here" "$work/refusals.log" <<'PY_REFUSALS'
import pathlib, sys
sys.path.insert(0, sys.argv[1])
from check_volatile_reduction_asm import ir_accesses
text = pathlib.Path(sys.argv[2]).read_text()
for symbol in ("atomic_product", "signed_sum", "fp_sum", "stride_sum",
               "dynamic_address", "intermediate_store", "two_reads"):
    expected = 0 if symbol == "atomic_product" else 2 if symbol == "two_reads" else 1
    count = ir_accesses(text, symbol)
    if count != expected:
        raise SystemExit(f"unexpected rewrite of {symbol}: {count} volatile loads")
PY_REFUSALS
# Explicit TLS overrides must reach the middle end, not just the backend.
for model in initial-exec global-dynamic local-dynamic; do
    CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=tls_product \
        "$CCC" -O3 -fPIC -ftls-model="$model" -S "$src" -o "$work/model.s" 2>"$work/model.log"
    python3 - "$here" "$work/model.log" "$model" <<'PY_MODEL'
import pathlib, sys
sys.path.insert(0, sys.argv[1])
from check_volatile_reduction_asm import ir_accesses
expected = 5 if sys.argv[3] == "initial-exec" else 1
actual = ir_accesses(pathlib.Path(sys.argv[2]).read_text(), "tls_product")
if actual != expected:
    raise SystemExit(f"TLS model {sys.argv[3]}: expected {expected}, found {actual}")
PY_MODEL
done
cat >"$work/policy.c" <<'C_POLICY'
unsigned long long reduce(volatile unsigned long long *p, unsigned long long n, unsigned long long seed) {
    for (unsigned long long i=0;i<n;++i) seed += *p;
    return seed;
}
C_POLICY
for backend in lccc-arm lccc-riscv lccc-i686; do
    binary=$(dirname "$CCC")/$backend
    CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=reduce \
        "$binary" -O3 -S "$work/policy.c" -o "$work/$backend.s" 2>"$work/$backend.log"
    if grep -q '^==== IR after iter=0 volatile_reduce ====' "$work/$backend.log"; then
        echo "FAIL: x86-64-only pass entered on $backend" >&2
        exit 1
    fi
done
# Smoke-test the new measurement ABI/oracle in CI; no timing thresholds.
"$GCC" -O3 -Wall -Wextra -Werror "$here/../benchmark/volatile/volatile_reduce_driver.c" -ldl -o "$work/matrix-driver"
for width in 32 64; do
    for kind in 0 1 2 3 4; do
        for address in 0 1; do
            "$CCC" -O3 -Wall -Wextra -Werror -fPIC -shared \
                -DWIDTH=$width -DKIND=$kind -DADDRESS=$address -DPRESSURE=1 \
                "$here/../benchmark/volatile/volatile_reduce_matrix.c" -o "$work/matrix.so"
            for n in 0 1 2 3 4 5 7 8 9 17 1024; do
                "$work/matrix-driver" "$work/matrix.so" "$n" 16 "$kind" "$width" 1 "$address" >"$work/matrix.out"
            done
        done
    done
done
printf '%s\n' 'volatile unsigned reductions: PASS'
# Dynamic TLS must also retain the original access semantics after lowering
# the retained original loads through the shared-object descriptor model.
cat >"$work/dynamic.c" <<'C'
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
int main(int argc, char **argv) {
    if (argc != 2) return 1;
    void *library = dlopen(argv[1], RTLD_NOW);
    if (!library) return 2;
    volatile uint64_t *slots = dlsym(library, "volatile_reduction_slots");
    uint64_t (*product)(uint64_t,uint64_t) = dlsym(library, "tls_product");
    if (!slots || !product) return 3;
    slots[1] = UINT64_C(0x9e3779b97f4a7c15);
    uint64_t expected = 7, hash = 0;
    for (uint64_t n=0;n<33;++n) {
        uint64_t actual=product(n,7);
        if (actual != expected) return 4;
        hash ^= actual;
        expected *= slots[1];
    }
    printf("%016llx\n", (unsigned long long)hash);
    return dlclose(library) != 0;
}
C
"$GCC" -O3 "$work/dynamic.c" -o "$work/dynamic" -ldl
"$CCC" -O3 -fPIC -shared "$src" -o "$work/dynamic.so"
timeout 30 "$work/dynamic" "$work/dynamic.so" >"$work/dynamic.out"
printf '%s\n' 'dynamic TLS unsigned reduction: PASS'
