#!/usr/bin/env bash
# Const-object read folding (from closed PR #816, re-gated for this tree):
# a read of an element or member of a CONST, relocation-free, default-
# address-space, non-TLS global folds to the value the program loads
# (`static const int g[4] = {...}; g[2]` becomes an immediate, exactly what
# GCC 16 emits), while every object whose storage can differ from its
# emitted initializer keeps its load.
#
# The exclusions are the load-bearing part.  The port's first form imaged
# EVERY non-volatile global; HEAD's lowering calls `eval_const_expr`
# opportunistically on runtime expressions, so a mutable `static __thread
# unsigned long arr[4]` folded to its zero image and the whole
# `arr[0]+..+arr[3]` sum vanished (tls_local_exec_direct, 19 siblings).
# This gate pins both directions so neither the fold nor its gating rots:
#   1. fold fires: readg() is immediate-only - no load of g or s remains;
#   2. mutable static: reads stay loads (stores must be observed);
#   3. TLS: reads of a zero-initialized __thread array stay %fs loads;
#   4. volatile const: the access is performed (C11 6.7.3), load stays;
#   5. runtime: all four programs print GCC-identical values (differential
#      against the $GCC oracle when one is available, else fixed expects).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC=$(readlink -f "$CCC")
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

fail() { echo "FAIL: check_const_object_fold: $*" >&2; exit 1; }

# ---- 1+5: the fold fires on const element/member reads ---------------------
cat >"$td/fold.c" <<'EOF'
#include <stdio.h>
static const int g[4] = {10, 20, 30, 40};
struct S { int a; int b; };
static const struct S s = {7, 9};
__attribute__((noinline)) int readg(void) { return g[2] + s.b; }
int main(void) { printf("%d\n", readg()); return 0; }
EOF
"$CCC" -O2 -S "$td/fold.c" -o "$td/fold.s"
body=$(sed -n '/^readg:/,/ret/p' "$td/fold.s")
grep -q '\$39' <<<"$body" \
    || fail "readg() did not fold g[2]+s.b to \$39:
$body"
grep -Eq '(g|s)[+(]' <<<"$body" \
    && fail "readg() folded but still loads from the object:
$body"
"$CCC" -O2 "$td/fold.c" -o "$td/fold" && out=$("$td/fold")
[ "$out" = "39" ] || fail "folded binary printed '$out', want 39"

# ---- 2: a MUTABLE static's reads stay loads --------------------------------
cat >"$td/mut.c" <<'EOF'
#include <stdio.h>
static int m[4] = {1, 2, 3, 4};
__attribute__((noinline)) void bump(void) { m[2] += 40; }
__attribute__((noinline)) int readm(void) { return m[2]; }
int main(void) { bump(); printf("%d\n", readm()); return 0; }
EOF
"$CCC" -O2 -S "$td/mut.c" -o "$td/mut.s"
mbody=$(sed -n '/^readm:/,/ret/p' "$td/mut.s")
grep -q 'm+8' <<<"$mbody" \
    || fail "readm() must LOAD the mutable m[2], got:
$mbody"
"$CCC" -O2 "$td/mut.c" -o "$td/mut" && out=$("$td/mut")
[ "$out" = "43" ] || fail "mutable-static binary printed '$out', want 43"

# ---- 3: TLS reads stay segment loads ----------------------------------------
cat >"$td/tls.c" <<'EOF'
#include <stdio.h>
static __thread unsigned long t[2];
__attribute__((noinline)) void setT(unsigned long v) { t[1] = v; }
__attribute__((noinline)) unsigned long readT(void) { return t[0] + t[1]; }
int main(void) { setT(41); printf("%lu\n", readT()); return 0; }
EOF
"$CCC" -O2 -S "$td/tls.c" -o "$td/tls.s"
tbody=$(sed -n '/^readT:/,/ret/p' "$td/tls.s")
grep -q '%fs:t@TPOFF' <<<"$tbody" \
    || fail "readT() must read TLS through %fs, got:
$tbody"
"$CCC" -O2 "$td/tls.c" -o "$td/tls" && out=$("$td/tls")
[ "$out" = "41" ] || fail "TLS binary printed '$out', want 41"

# ---- 4: a volatile-qualified ACCESS to a const object is performed ---------
cat >"$td/vol.c" <<'EOF'
#include <stdio.h>
static const int g[4] = {10, 20, 30, 40};
int main(void) {
    int a = *(volatile const int *)&g[1];
    printf("%d\n", a);
    return 0;
}
EOF
"$CCC" -O2 -S "$td/vol.c" -o "$td/vol.s"
grep -q 'g+4' "$td/vol.s" \
    || fail "volatile-qualified read of g[1] was folded away (C11 6.7.3: the access must be performed)"
"$CCC" -O2 "$td/vol.c" -o "$td/vol" && out=$("$td/vol")
[ "$out" = "20" ] || fail "volatile-access binary printed '$out', want 20"

echo "check_const_object_fold: PASSED (fold fires on const, loads survive on mutable/TLS/volatile)"
