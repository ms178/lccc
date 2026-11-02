#!/usr/bin/env bash
# Integer-parameter provenance: laundered pointers must not present Param roots.
#
# WHY THIS EXISTS
#
# The frontend erases integer<->pointer conversions (`emit_implicit_cast`
# returns the source operand unchanged when either side is Ptr), so
# `(char *)raw` for an integer `raw` lowers to a GEP *directly on*
# `ParamRef { ty: U64 }` — no Cast marks the laundering. The B3 freshness
# rule (param-vs-alloca disjoint) briefly trusted every ParamRef, and the
# vectorizer emitted an UNGUARDED vector copy for
# `buf[i] = ((char *)raw)[i]` (negative-verified: the pre-fix tree emits the
# ymm loop with no overlap check for h64 below). The shared
# `src/ir/provenance.rs` leaf typing closes it: only pointer-typed ParamRefs
# root, so the laundered copy keeps its runtime guard (or memmove/scalar).
#
# This gate pins BOTH directions so the fix cannot be "achieved" by
# disabling the optimization:
#   h64 (integer param laundered to pointer): vector loop AND guard present.
#   g64 (genuine pointer param): vector loop present, NO guard (B3 intact).
# plus runtime bit-exactness vs the oracle on a benign in-bounds input.
#
# Usage:
#   tests/regression/check_provenance_int_param.sh
# Environment:
#   LCCC            compiler to test (default: target/fastbuild/lccc)
#   CCC_ORACLE_CC   reference compiler (default: cc, gcc or clang)
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-target/fastbuild/lccc}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: no compiler at $LCCC" >&2; exit 1; }
ORACLE=${CCC_ORACLE_CC:-}
if [[ -z "$ORACLE" ]]; then
    for c in cc gcc clang; do command -v "$c" >/dev/null 2>&1 && { ORACLE=$c; break; }; done
fi
[[ -n "$ORACLE" ]] || { echo "FAIL: no reference compiler found" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

pass=0; fail=0; skip=0
ok()   { pass=$((pass+1)); printf 'ok   %s\n' "$*"; }
bad()  { fail=$((fail+1)); printf 'FAIL %s\n' "$*"; }

cat > "$work/prov.c" <<'EOF_C'
typedef unsigned long u64;
static char sink[64];
static char input[64];
// Laundered: integer param converted to a pointer. The integer names no
// object, so the copy may overlap the alloca and must stay guarded.
void h64(u64 raw) {
    char buf[64];
    for (int i = 0; i < 64; i++) buf[i] = ((char *)raw)[i];
    for (int i = 0; i < 64; i++) sink[i] = buf[i];
}
// Genuine: pointer param vs local alloca. Provably disjoint (B3 freshness),
// so the copy must vectorize WITHOUT a guard.
void g64(char *src) {
    char buf[64];
    for (int i = 0; i < 64; i++) buf[i] = src[i];
    for (int i = 0; i < 64; i++) sink[i] = buf[i];
}
#ifndef NO_MAIN
#include <stdio.h>
int main(void) {
    for (int i = 0; i < 64; i++) input[i] = (char)(i * 3 + 1);
    h64((u64)input);
    unsigned c = 0;
    for (int i = 0; i < 64; i++) c = c * 33 + (unsigned char)sink[i];
    printf("h64 %08x\n", c);
    g64(input);
    c = 0;
    for (int i = 0; i < 64; i++) c = c * 33 + (unsigned char)sink[i];
    printf("g64 %08x\n", c);
    return 0;
}
#endif
EOF_C

"$LCCC" -O2 -march=x86-64-v3 -DNO_MAIN -S -o "$work/prov.s" "$work/prov.c" 2>/dev/null \
    || { bad "lccc could not compile prov.c"; echo; echo "provenance-int-param gate: PASS=$pass FAIL=$fail SKIP=$skip"; exit 1; }

# Guard jumps are UNSIGNED-compare jumps: the vectorizer's overlap check
# compares addresses, and the x86 backend (`cmp_jcc` in
# src/backend/x86/codegen/comparison.rs) maps Ult/Ule/Ugt/Uge to exactly
# jb/jbe/ja/jae. Signed loop-control jumps (jge/jl from the fixtures' `int`
# IVs) must NOT count — g64's rolled vector loops contain 8 of them, so the
# full conditional-jump class would false-fail. Counting the whole unsigned
# family (not just the jb|jae pair emitted today) also closes both
# reformulation hazards: a `ja`/`jbe` guard would otherwise false-pass g64
# and false-fail h64.
read -r h_vec h_guard g_vec g_guard < <(python3 - "$work/prov.s" <<'EOF_PY'
import re, sys
txt = open(sys.argv[1]).read()
def body(fn):
    m = re.search(rf'^{fn}:\n(.*?)^\s*\.size\s+{fn}\b', txt, re.S | re.M)
    return m.group(1) if m else ""
out = []
for fn in ("h64", "g64"):
    b = body(fn)
    vec = len(re.findall(r'^\s*v\w+\s+.*%ymm', b, re.M))
    guard = len(re.findall(r'^\s*(?:jbe|jae|jb|ja)\s+\.L', b, re.M))
    out.extend((vec, guard))
print(*out)
EOF_PY
)
if [[ "$h_vec" -ge 1 && "$h_guard" -ge 1 ]]; then
    ok "h64 laundered copy vectorizes guarded ($h_vec ymm memops, $h_guard guard jumps)"
else
    bad "h64 laundered copy must keep vector loop + guard (ymm=$h_vec guards=$h_guard)"
fi
if [[ "$g_vec" -ge 1 && "$g_guard" -eq 0 ]]; then
    ok "g64 genuine copy vectorizes unguarded ($g_vec ymm memops, B3 intact)"
else
    bad "g64 genuine copy must vectorize guard-free (ymm=$g_vec guards=$g_guard)"
fi

if "$ORACLE" -O2 -march=x86-64-v3 -o "$work/prov.oracle" "$work/prov.c" 2>/dev/null; then
    "$LCCC" -O2 -march=x86-64-v3 -o "$work/prov.lccc" "$work/prov.c" 2>/dev/null \
        || { bad "lccc could not link prov.c"; }
    "$work/prov.oracle" > "$work/o.txt" 2>&1
    "$work/prov.lccc" > "$work/l.txt" 2>&1
    if diff -q "$work/l.txt" "$work/o.txt" >/dev/null; then
        ok "runtime bit-exact vs $ORACLE on benign in-bounds input"
    else
        bad "runtime output differs from $ORACLE"
    fi
else
    bad "reference compiler could not build prov.c"
fi

echo
echo "provenance-int-param gate: PASS=$pass FAIL=$fail SKIP=$skip"
[[ "$fail" -eq 0 ]] || exit 1
exit 0
