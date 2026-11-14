#!/usr/bin/env bash
# The two byte-classifier loop spellings (tests/bench/k_namechars.c and
# k_classify.c) must BOTH reach the packed byte-count emitter at -O3:
#
#   * the counting form `if (a||b||...) n++;` needs if_convert's combine to
#     accept the chain — its profitability simulation must simulate the
#     caller's LATER combine rounds (the jump-threaded chain shares one
#     increment block, so no diamond/triangle converts until the chain is
#     one block), and `is_boolean_condition` must recurse through the
#     And/Or conditions earlier rounds produced;
#
#   * the summed-predicate form `c += pred(s[i]);` needs
#     `vectorize_byte_count_normalize`'s pure-value-triangle flatten (the
#     surviving ~10-instruction arm is rightly over the loop-resident cmov
#     budget, so only the transactional adapter may take it).
#
# Either regressing shows up here as a missing `psadbw`, not as a silent
# 3-10x runtime loss found one release later.  The negative row pins the
# fail-closed side: an arm with a STORE must not be flattened/speculated,
# and the loop must stay scalar.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo"

ccc=${CCC:-target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

cat >"$tmp/count.c" <<'EOF'
static char s[4096];
unsigned long long count_name_chars(void) {
    unsigned long long count = 0;
    for (unsigned long i = 0; i < 4096; i++) {
        char c = s[i];
        if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
            (c >= '0' && c <= '9') || c == '_' || c == '-' || c == '.')
            count++;
    }
    return count;
}
EOF

cat >"$tmp/sum.c" <<'EOF'
static char s[4096];
static int is_name_char(char ch) {
    return ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
            (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.');
}
unsigned long long sum_name_chars(void) {
    unsigned long long c = 0;
    for (int i = 0; i < 4096; i++) {
        c += (unsigned)is_name_char(s[i]);
    }
    return c;
}
EOF

# Fail-closed row: the arm writes memory, so neither if-conversion nor the
# adapter's flatten may speculate it; the loop must stay scalar.
cat >"$tmp/sideeffect.c" <<'EOF'
static char s[4096];
static unsigned long hits[4096];
unsigned long long count_with_store(void) {
    unsigned long long count = 0;
    for (unsigned long i = 0; i < 4096; i++) {
        char c = s[i];
        if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) {
            hits[i] = i;
            count++;
        }
    }
    return count;
}
EOF

"$ccc" -O3 -march=x86-64-v3 -S "$tmp/count.c" -o "$tmp/count.s"
"$ccc" -O3 -march=x86-64-v3 -S "$tmp/sum.c" -o "$tmp/sum.s"
"$ccc" -O3 -march=x86-64-v3 -S "$tmp/sideeffect.c" -o "$tmp/sideeffect.s"

grep -Eq '(^|[[:space:]])v?psadbw' "$tmp/count.s" ||
    fail "the counting form (if (a||b||...) n++) no longer reaches the packed byte-count emitter"
grep -Eq '(^|[[:space:]])v?psadbw' "$tmp/sum.s" ||
    fail "the summed-predicate form (c += pred(s[i])) no longer reaches the packed byte-count emitter"
if grep -Eq '(^|[[:space:]])v?psadbw' "$tmp/sideeffect.s"; then
    fail "a loop whose arm STORES was packed: the speculation gates are broken"
fi

# SSE2 baseline must hold the counting form too (psadbw, 16 lanes).
"$ccc" -O3 -S "$tmp/count.c" -o "$tmp/count_sse2.s"
grep -Eq '(^|[[:space:]])v?psadbw' "$tmp/count_sse2.s" ||
    fail "the counting form lost the SSE2-baseline packed lowering"

echo "PASS: both classifier spellings reach psadbw; the store arm stays scalar"
