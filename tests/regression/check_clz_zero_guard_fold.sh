#!/usr/bin/env bash
# Codegen gate for the guarded-clz zero-guard elimination.
#
# `x ? __builtin_clz(x) : 32` must collapse onto the Clz intrinsic (whose IR
# semantics already define Clz(0) == 32). Before the fix, codegen emitted the
# intrinsic's own zero fix-up AND a second select materialization
# (`movq $32, %rdx; cmovneq %rax, %rdx`) — a fully dead tail. The gate checks
# the function body for the select residue and for the bsr core.
# Runtime semantics are covered differentially by guarded_clz_ctz_ternary.c.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/c.c" <<'EOF'
int lz(unsigned x) { return x ? __builtin_clz(x) : 32; }
EOF

"$CCC" -O2 -S "$td/c.c" -o "$td/c.s"
body=$(sed -n '/^lz:/,/^\.size[[:space:]]*lz/p' "$td/c.s")

if ! grep -qE '\bbsrl?\b|\blzcntl\b' <<<"$body"; then
    echo "FAIL: lz() should contain a bsr (or lzcnt) core; got:" >&2
    echo "$body" >&2
    exit 1
fi
# Select residue from the pre-fix duplicated zero guard: a second materialized
# `movq $32` + cmov (and the double test of the operand) must be gone.
if grep -qE '\bcmov' <<<"$body" || grep -qE 'movq[[:space:]]+\$32,' <<<"$body"; then
    echo "FAIL: lz() still contains the redundant zero-guard select tail:" >&2
    echo "$body" >&2
    exit 1
fi
n_test=$(grep -cE '\btest[blqw]?\b' <<<"$body" || true)
if [ "$n_test" -gt 1 ]; then
    echo "FAIL: lz() tests its operand more than once (doubled zero guard):" >&2
    echo "$body" >&2
    exit 1
fi

# === NonZero-arm guard-select discipline (the speculation hole) ===
#
# The collapse above is an algebraic identity ONLY for the defined-zero
# intrinsics. `ClzNonZero`/`CtzNonZero` (created by CVP under a dominating
# nonzero proof) have no value at zero, and if-conversion can speculate one
# ABOVE its proof: the branchy guarded form survives to CVP whenever no
# pre-CVP diamond conversion ran (i686 always — its pipeline has no
# vectorizer pre-conversion; x86-64 whenever the vectorizer is disabled or
# declines the diamond), CVP specializes the arm under the branch proof, and
# if-conversion then emits `Select(x, ClzNonZero(x), 32)`. Collapsing that
# select returned BSR's architecturally-undefined destination for x == 0
# (measured: clz_if(0) == -846929913). The select must survive; only CVP's
# own fact-stack folding may remove it, and only under a dominating proof.
cat >"$td/branchy.c" <<'EOF'
#include <stdio.h>
#include <stdlib.h>
static int clz_if(unsigned x) {
    int t;
    if (x) { t = __builtin_clz(x); } else { t = 32; }
    return t;
}
static int ctz_if(unsigned x) {
    int t;
    if (x) { t = __builtin_ctz(x); } else { t = 32; }
    return t;
}
static int clz_if64(unsigned long long x) {
    int t;
    if (x) { t = __builtin_clzll(x); } else { t = 64; }
    return t;
}
static int ctz_if64(unsigned long long x) {
    int t;
    if (x) { t = __builtin_ctzll(x); } else { t = 64; }
    return t;
}
int main(void) {
    /* Call through volatile pointers: inlining the guarded form into the
     * checking loop changes the arm shapes (the `bad = 1` store makes the
     * arms impure, if-conversion declines, and the hazard never fires).
     * The standalone function IS the miscompiling shape; keep it emitted.
     * References are shift-loop spellings: they contain no bitcount
     * intrinsic, so a bitcount-guard regression cannot make the oracle
     * agree with a miscompile by miscompiling the same way. */
    int (*volatile pclz)(unsigned) = clz_if;
    int (*volatile pctz)(unsigned) = ctz_if;
    int (*volatile pclz64)(unsigned long long) = clz_if64;
    int (*volatile pctz64)(unsigned long long) = ctz_if64;
    static const unsigned cases[] = {
        0u, 1u, 2u, 3u, 0x80000000u, 0xFFFFFFFFu, 0x00010000u, 0x00FFFF00u,
    };
    int bad = 0;
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        unsigned x = cases[i];
        unsigned s = x;
        int e = 32;
        while (s) { e--; s >>= 1; }
        int f = 0;
        s = x;
        while (s && !(s & 1u)) { f++; s >>= 1; }
        if (!x) f = 32;
        if (pclz(x) != e) { printf("clz_if(%u)=%d want %d\n", x, pclz(x), e); bad = 1; }
        if (pctz(x) != f) { printf("ctz_if(%u)=%d want %d\n", x, pctz(x), f); bad = 1; }
    }
    static const unsigned long long c64[] = { 0ull, 1ull, 2ull, 0x8000000000000000ull,
                                               0xFFFFFFFFFFFFFFFFull, 0x0000000100000000ull };
    for (size_t i = 0; i < sizeof c64 / sizeof c64[0]; i++) {
        unsigned long long x = c64[i];
        unsigned long long s = x;
        int e = 64;
        while (s) { e--; s >>= 1; }
        int f = 0;
        s = x;
        while (s && !(s & 1u)) { f++; s >>= 1; }
        if (!x) f = 64;
        if (pclz64(x) != e) { printf("clz_if64(%llu)=%d want %d\n", x, pclz64(x), e); bad = 1; }
        if (pctz64(x) != f) { printf("ctz_if64(%llu)=%d want %d\n", x, pctz64(x), f); bad = 1; }
    }
    return bad;
}
EOF

# Topology 1: default pipeline (x86-64 pre-conversion hides the shape, but
# pin it anyway — any future pipeline reorder re-exposes it).
"$CCC" -O2 "$td/branchy.c" -o "$td/branchy" || { echo "FAIL: branchy build" >&2; exit 1; }
"$td/branchy" || { echo "FAIL: branchy form miscompiles (default pipeline)" >&2; exit 1; }
# Topology 2: no pre-CVP diamond conversion — the exact shape that
# miscompiled. CCC_DISABLE_PASSES=vectorize removes the vectorizer's
# embedded if-conversion fixpoint, so the branchy diamond reaches CVP.
CCC_DISABLE_PASSES=vectorize "$CCC" -O2 "$td/branchy.c" -o "$td/branchy_nc" \
    || { echo "FAIL: branchy build (no pre-conversion)" >&2; exit 1; }
"$td/branchy_nc" || { echo "FAIL: branchy form miscompiles (no pre-conversion)" >&2; exit 1; }

# Topology 3: i686 — always takes the no-pre-conversion path, and cannot be
# executed in every environment (multilib). Pin the ASSEMBLY contract
# instead: a bare bsr/bsf with no zero handling is the miscompile shape; a
# correct lowering must also test the operand (branchy select or cmov).
CCC686=${CCC686:-./target/fastbuild/lccc-i686}
if [ -x "$CCC686" ]; then
    cat >"$td/i686.c" <<'EOF'
int clz_if(unsigned x) {
    int t;
    if (x) { t = __builtin_clz(x); } else { t = 32; }
    return t;
}
EOF
    "$CCC686" -O2 -m32 -S "$td/i686.c" -o "$td/i686.s" \
        || { echo "FAIL: i686 compile" >&2; exit 1; }
    ibody=$(sed -n '/^clz_if:/,/ret/p' "$td/i686.s")
    if grep -qE '\bbsrl\b' <<<"$ibody" && ! grep -qE '\btest[blqw]?\b' <<<"$ibody"; then
        echo "FAIL: i686 clz_if is a bare bsr with no zero test (returns" \
             "the undefined BSR destination for x==0):" >&2
        echo "$ibody" >&2
        exit 1
    fi
fi
echo "OK clz_zero_guard_fold"
