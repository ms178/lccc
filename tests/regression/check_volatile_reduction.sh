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
for flags in '-O3 -fno-pic -no-pie' '-O3 -fPIE -pie' '-O3 -fPIC' '-O3 -g -fno-pic -no-pie' '-O2' '-Os'; do
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
python3 - "$work/on.s" "$work/off.s" <<'PY'
import re, sys

def accesses(path):
    text = open(path).read()
    body = text.split('tls_product:', 1)[1].split('.size tls_product', 1)[0]
    return len(re.findall(r'%fs:volatile_reduction_slots@TPOFF\+8', body))

assert accesses(sys.argv[1]) == 5, 'missing four-lane main loop or scalar tail'
assert accesses(sys.argv[2]) == 1, 'disable control did not retain rolled loop'
PY
printf '%s\n' 'volatile unsigned reductions: PASS'
# Dynamic TLS must also retain the original access semantics after lowering
# the four cloned loads through the shared-object descriptor model.
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
