#!/usr/bin/env bash
# Pointer -> float casts must be VALUE conversions (pointer as unsigned
# integer), not bit-reinterprets. Two bugs were fixed:
#  1. The frontend dropped `(double)p` entirely (both 8 bytes), so the ABI
#     return path bitcast the pointer (`movq %reg, %xmm0`), printing 0.0 for a
#     0x55... address.
#  2. The x86 register-direct int->float fast path treated Ptr as signed
#     (`cvtsi2sdq`), so a high-bit address became negative instead of taking
#     the U64 shift+round dance.
# The direct cast must equal the standard-compliant `(double)(uintptr_t)p`.
set -uo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
#include <stdint.h>
typedef float f32; typedef double f64;
__attribute__((noinline)) f64 d1(void *p) { return (double)p; }
__attribute__((noinline)) f64 d2(void *p) { return (double)(uintptr_t)p; }
__attribute__((noinline)) f32 f1(void *p) { return (float)p; }
__attribute__((noinline)) f32 f2(void *p) { return (float)(uintptr_t)p; }
int main(void) {
    /* low, high-bit (kernel-style), and canonical-heap addresses */
    void *addrs[3] = { (void*)0x55ULL, (void*)0xffffffff80000000ULL, (void*)0x7ffffffff000ULL };
    for (int i = 0; i < 3; i++) {
        if (d1(addrs[i]) != d2(addrs[i])) return 1;
        if (f1(addrs[i]) != f2(addrs[i])) return 2;
    }
    return 0;
}
EOF
"$CCC" -O2 "$td/test.c" -o "$td/lccc_bin"
"$td/lccc_bin"; l=$?
if [ "$l" -ne 0 ]; then
    echo "ptr->float cast mismatch (direct vs uintptr_t): rc=$l"
    exit 1
fi
# The high-bit direct cast must NOT be a plain bitcast/signed convert:
# `movq %reg, %xmm` (bitcast) or a bare cvtsi2sdq without the U64 dance is wrong.
"$CCC" -O2 -S "$td/test.c" -o "$td/test.s"
body=$(sed -n '/^d1:/,/^\.size d1/p' "$td/test.s")
if grep -Eq 'movq[[:space:]]+%r[a-z0-9]+, %xmm' <<<"$body" && ! grep -Eq 'cvtsi2sdq' <<<"$body"; then
    echo "ptr->f64 is still a bitcast"
    echo "--- $body"
    exit 1
fi
