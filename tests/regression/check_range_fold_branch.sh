#!/usr/bin/env bash
# Emission + semantics gate for the BRANCH form of range-check folding
# (range_fold's fold_branch_chains).
#
#   fire-shape    `while (x >= 7 && x <= 29)` — a loop guard that must stay
#                 a branch chain (the backedge pins it) — fuses to
#                 sub/lea -7 + ONE unsigned compare against the span 22;
#                 the compare-with-7 must be gone.
#   digit-shape   the csv_field_sum digit test over a big body
#                 (if-conversion declines) — fuses to sub/lea -48 + one
#                 unsigned byte compare against 9.
#   nofire-shape  mixed signedness ((unsigned)x >= 48u && x <= 57) — the
#                 two compares live in different domains; BOTH must remain
#                 (a fold here would misclassify negative x).
#   span-shape    a span that does not fit the compare width
#                 (x >= INT_MIN && x <= INT_MAX) — unfolded, both compares
#                 remain.
#
# Every configuration is also EXECUTED and diffed against -O0: a gate
# that only checked emission would pass while silently breaking the
# loop it rewrote.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
src=$repo/tests/regression/range_fold_branch.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
check_eq() { # <desc> <actual> <expected>
    if [[ "$2" != "$3" ]]; then
        echo "FAIL: $1 -- got '$2', want '$3'" >&2
        fail=1
    fi
}
check_absent() { # <desc> <file> <egrep-pattern>
    if grep -qE "$3" "$2"; then
        echo "FAIL: $1 -- forbidden pattern '$3' matched:" >&2
        grep -nE "$3" "$2" | head -3 >&2
        fail=1
    fi
}
check_present() { # <desc> <file> <egrep-pattern>
    if ! grep -qE "$3" "$2"; then
        echo "FAIL: $1 -- required pattern '$3' not found" >&2
        fail=1
    fi
}

# --- 1. the loop-guard fire shape -------------------------------------------
cat > "$tmp/fire.c" <<'EOF'
int guard(int x) {
    int n = 0;
    while (x >= 7 && x <= 29) { x -= 3; n++; }
    return n;
}
int main(void) {
    /* 11 -> 8 -> 5(stop): two; 29 down to 5: eight; 7 -> 4: one. */
    return (guard(11) == 2 && guard(29) == 8 && guard(7) == 1 &&
            guard(6) == 0 && guard(30) == 0) ? 0 : 1;
}
EOF
"$ccc" -O2 -S "$tmp/fire.c" -o "$tmp/fire.s"
# The compare-with-7 must be gone (the fused form subtracts 7 instead).
check_absent "loop-guard: compare-with-7 eliminated" "$tmp/fire.s" 'cmp[a-z]*\s+\$7,'
# The fused subtract and the unsigned span compare must be there.
check_present "loop-guard: bias subtract present" "$tmp/fire.s" 'sub[a-z]*\s+\$7,|lea[a-z]*\s+-7\('
check_present "loop-guard: span compare present" "$tmp/fire.s" 'cmp[a-z]*\s+\$22,'
# Exactly two conditional branches remain in `guard` itself (the rotated
# entry check and the latch back-edge) — extract the function body (from
# its label to .cfi_endproc) and count there, not in main's verification
# chain.
sed -n '/^guard:/,/^\.cfi_endproc/p' "$tmp/fire.s" > "$tmp/guard-body.s"
n_chain=$(grep -cE '\bj(a|be|ae|b|g|le|l|ge|ne|e)\b' "$tmp/guard-body.s" || true)
if [[ "$n_chain" -gt 3 ]]; then
    echo "FAIL: loop-guard: too many conditional branches in guard ($n_chain) -- the chain did not fuse" >&2
    fail=1
fi
"$ccc" -O2 "$tmp/fire.c" -o "$tmp/fire.bin" && "$tmp/fire.bin"
check_eq "loop-guard: execution" "$?" "0"

# --- 2. the digit fire shape (big body; if-conversion declines) -------------
cat > "$tmp/digit.c" <<'EOF'
static void step(int *a) {
    *a += 1; *a ^= 0x5555; *a += 2; *a ^= 0xaaaa;
    *a += 3; *a ^= 0x3333;
}
int dig(char c, int *acc) {
    if (c >= '0' && c <= '9') {
        step(acc);
        return 1;
    }
    return 0;
}
/* The loop-guard form inside a real loop: the local-variable spelling
 * (the csv_field_sum shape). Must fuse to one bias subtract + one
 * unsigned span compare per character. */
int count_digs(const char *s) {
    int r = 0;
    for (const char *p = s; *p; p++) {
        char c = *p;
        if (c >= '0' && c <= '9') r++;
    }
    return r;
}
int main(void) {
    int acc = 0, acc_ref = 0;
    int r = 0;
    const char *s = "a0z-9+_8";
    for (const char *p = s; *p; p++) {
        r += dig(*p, &acc);
        if (*p >= '0' && *p <= '9') step(&acc_ref);
    }
    return (r == 3 && acc == acc_ref && count_digs(s) == 3) ? 0 : 1;
}
EOF
"$ccc" -O2 -S "$tmp/digit.c" -o "$tmp/digit.s"
# Extract dig's body: main's *p re-load spelling legitimately keeps its
# compare (two source-level loads of *p are not CSE'd before range_fold —
# a separate load-CSE follow-up), so the absent check is scoped to dig
# and count_digs, whose spelling the fold owns.
sed -n '/^dig:/,/^\.cfi_endproc/p' "$tmp/digit.s" > "$tmp/dig-body.s"
check_absent "digit: compare-with-48 eliminated (dig)" "$tmp/dig-body.s" 'cmp[a-z]*\s+\$48,'
check_present "digit: bias subtract present" "$tmp/dig-body.s" 'sub[a-z]*\s+\$48,|lea[a-z]*\s+-48\('
check_present "digit: span compare present" "$tmp/dig-body.s" 'cmp[a-z]*\s+\$9,'
# The in-loop local-variable spelling must ALSO fuse: extract count_digs
# and require the bias form there (the And-of-compares path).
sed -n '/^count_digs:/,/^\.cfi_endproc/p' "$tmp/digit.s" > "$tmp/count_digs-body.s"
check_absent "digit: loop form compare-with-48 eliminated" "$tmp/count_digs-body.s" 'cmp[a-z]*\s+\$48,'
check_present "digit: loop form bias subtract present" "$tmp/count_digs-body.s" 'sub[a-z]*\s+\$48,|lea[a-z]*\s+-48\('
"$ccc" -O2 "$tmp/digit.c" -o "$tmp/digit.bin" && "$tmp/digit.bin"
check_eq "digit: execution" "$?" "0"

# --- 3. no-fire: mixed signedness -------------------------------------------
cat > "$tmp/mix.c" <<'EOF'
int mix(int c) {
    if ((unsigned)c >= 48u && c <= 57) return 1;
    return 0;
}
int main(void) {
    /* -1: (unsigned)(-1) >= 48u is true, but -1 <= 57 is also true ->
       1. -100: unsigned huge >= 48u true, -100 <= 57 true -> 1.
       58: >= 48u true, <= 57 false -> 0. */
    return (mix(-1) == 1 && mix(-100) == 1 && mix(58) == 0 && mix(48) == 1) ? 0 : 1;
}
EOF
"$ccc" -O2 -S "$tmp/mix.c" -o "$tmp/mix.s"
check_present "mixed-sign: both compares remain" "$tmp/mix.s" 'cmp[a-z]*\s+\$48,'
check_present "mixed-sign: upper compare remains" "$tmp/mix.s" 'cmp[a-z]*\s+\$57,'
"$ccc" -O2 "$tmp/mix.c" -o "$tmp/mix.bin" && "$tmp/mix.bin"
check_eq "mixed-sign: execution" "$?" "0"

# --- 4. no-fire: unrepresentable span ----------------------------------------
cat > "$tmp/span.c" <<'EOF'
#include <limits.h>
int span(int x) {
    if (x >= INT_MIN && x <= INT_MAX) return 1;
    return 0;
}
int main(void) { return (span(0) == 1 && span(-1) == 1) ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/span.c" -o "$tmp/span.s"
# The span 0xFFFFFFFF is not representable in the I32 compare domain, so
# the fold must leave the predicate alone (SCCP may legitimately fold the
# tautological compares to constants — what must NEVER appear is the
# unsigned-bias shape with a wrapped span).
check_absent "span-overflow: no wrapped-span fold" "$tmp/span.s" 'sub[a-z]*\s+\$-214748364|cmp[a-z]*\s+\$-1,|cmp[a-z]*\s+\$4294967295,'
"$ccc" -O2 "$tmp/span.c" -o "$tmp/span.bin" && "$tmp/span.bin"
check_eq "span-overflow: execution" "$?" "0"

# --- 5. the full battery, differential across optimization levels -----------
for opt in -O0 -O1 -O2 -O3 -Os; do
    "$ccc" "$opt" "$src" -o "$tmp/battery$opt.bin"
    "$tmp/battery$opt.bin" > "$tmp/battery$opt.out" 2>&1
    check_eq "battery $opt exit" "$?" "0"
done
for opt in -O1 -O2 -O3 -Os; do
    if ! diff -q "$tmp/battery-O0.out" "$tmp/battery$opt.out" >/dev/null; then
        echo "FAIL: battery $opt output differs from -O0:" >&2
        diff "$tmp/battery-O0.out" "$tmp/battery$opt.out" | head -5 >&2
        fail=1
    fi
done

exit $fail
