#!/usr/bin/env bash
# Predicate materialization: `c ? 1 : 0` is `c != 0`, and `c ? 0 : 1` is
# `c == 0`.
#
# C spells a boolean result two ways.  `return a != b;` lowers to the
# comparison itself -- one `setcc` off the flags the compare already set.  The
# equivalent `return a != b ? 1 : 0;` lowers to a Select over the two
# constants, and a Select needs its arms in registers: x86 emitted
# `mov $1,%ecx; mov $0,%eax; cmovne` where the comparison alone is
# `xor %eax,%eax; setne %al`.  Both oracles canonicalize the two spellings to
# the same IR, so this gate pins the rewrite that does the same.
#
# The same family covers the mask merges `(a != 0) | (b != 0)` -> `(a | b) != 0`
# (nine instructions down to four) and `(a == 0) & (b == 0)` -> `(a | b) == 0`.
# The two MIXED forms are not rewrites at all -- `(a == 0) | (b == 0)` is about
# two separate zeros, and `(a != 0) & (b != 0)` is false for every pair that
# shares no set bit (`a = 1, b = 2`) -- so they are pinned as refusals.
#
# Six things are asserted, because an instruction-count win that changes
# answers is worthless:
#
#   1. every folded shape loses its `cmov` and reaches the oracle count,
#   2. the refusals keep their exact counts in both builds,
#   3. `CCC_NO_PREDICATE_MATERIALIZE=1` restores every count and brings the
#      `cmov` back, which is what proves the assertions above belong to this
#      fold,
#   4. the other backends never get *longer* (i686, AArch64, RISC-V),
#   5. -O0, -O1, -O2, -O3, -Os, the kill switch and GCC agree bit for bit on a
#      checksum over the shapes,
#   6. the vectorizer's mask store stays correct: a loop that stores
#      `lo <= c && c <= hi ? 1 : 0` is vectorized once the condition is a
#      comparison, and a packed compare is 0/-1 per lane while the C program
#      stores 0/1.  The element-wise check sweeps every trip count around the
#      vector width, which is where a missing `& 1` shows up.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
bindir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
tmp=${TMPDIR:-/tmp}/lccc-predicate-materialize.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
typedef unsigned u32;
typedef unsigned long u64;

int from_cmp(int a, int b)      { return a != b ? 1 : 0; }
int from_cmp_neg(int a, int b)  { return a != b ? 0 : 1; }
int from_lt(int a, int b)       { return a < b ? 1 : 0; }
int from_lt_neg(int a, int b)   { return a < b ? 0 : 1; }
u32 zero_or_one(int c)          { return c ? 1u : 0u; }
int one_or_zero(int c)          { return c ? 0 : 1; }
u32 mask_of_sign(int x)         { return x < 0 ? 0u : 1u; }
u64 zero_or_one64(long long c)  { return c ? 1ull : 0ull; }
/* Mask merges: a disjunction is zero exactly when both halves are. */
int or_or(int a, int b)         { return (a != 0) | (b != 0); }
int and_eq(int a, int b)        { return (a == 0) & (b == 0); }
int or3(int a, int b, int c)    { return (a != 0) | (b != 0) | (c != 0); }

/* Controls: not a boolean materialization, and not one of the two valid
   merges.  Each keeps its count in BOTH builds. */
int keep_and_ne(int a, int b)   { return (a != 0) & (b != 0); }
int keep_or_eq(int a, int b)    { return (a == 0) | (b == 0); }
int keep_xor(int a, int b)      { return (a != 0) ^ (b != 0); }
int keep_three_two(int c)       { return c ? 3 : 2; }
int keep_one_two(int c)         { return c ? 1 : 2; }
int g_shared;
int keep_shared(int a, int b)   { int t = a != b; g_shared = t; return t ? 0 : 1; }
C

"$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/on.s"
CCC_NO_PREDICATE_MATERIALIZE=1 "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/off.s"

fn_body() { # $1 = asm file, $2 = label; body between the label and the return
    sed -n "/^$2:/,/^[[:space:]]*\(ret\|retq\|ldp x29, x30\)/p" "$1"
}
count() { # $1 = asm file, $2 = label
    fn_body "$1" "$2" | grep -c -e '^[[:space:]]\+[a-z]' || true
}
body() { fn_body "$1" "$2"; }
expect() { # $1 = label, $2 = expected count, $3 = build
    local n
    n=$(count "$tmp/$3" "$1")
    if [ "$n" -ne "$2" ]; then
        echo "$1 costs $n instructions in $3, expected $2:" >&2
        body "$tmp/$3" "$1" >&2
        exit 1
    fi
}
want() { # $1 = build, $2 = label, $3 = pattern
    if ! grep -Eq -- "$3" <<<"$(body "$tmp/$1" "$2")"; then
        echo "$2 in $1 has no '$3':" >&2
        body "$tmp/$1" "$2" >&2
        exit 1
    fi
}
refuse() { # $1 = build, $2 = label, $3 = pattern, $4 = message
    if grep -Eq -- "$3" <<<"$(body "$tmp/$1" "$2")"; then
        echo "$4: $2 in $1 still contains '$3':" >&2
        body "$tmp/$1" "$2" >&2
        exit 1
    fi
}

# ── 1. The folded shapes reach the comparison, without a select arm ────────
expect from_cmp 4 on.s
expect from_cmp_neg 4 on.s
expect from_lt 4 on.s
expect from_lt_neg 4 on.s
expect zero_or_one 4 on.s
expect one_or_zero 4 on.s
expect mask_of_sign 4 on.s
expect zero_or_one64 4 on.s
expect or_or 4 on.s
expect and_eq 4 on.s
expect or3 5 on.s
for label in from_cmp from_cmp_neg from_lt from_lt_neg zero_or_one one_or_zero \
             mask_of_sign zero_or_one64 or_or and_eq or3; do
    refuse on.s "$label" 'cmov' "the select must be gone"
done
# The boolean itself is a single `setcc` on a zeroed register, not an
# arm-pair move: the comparison's own flags are what is tested.
want on.s from_cmp 'setne[[:space:]]'
want on.s from_cmp_neg 'sete[[:space:]]'
want on.s one_or_zero 'sete[[:space:]]'
want on.s mask_of_sign 'setge[[:space:]]'
# The merges are one disjunction followed by one compare.
want on.s or_or 'or[lq][[:space:]]'
want on.s and_eq 'or[lq][[:space:]]'
refuse on.s or_or 'setne.*setne' "the two selectors must have merged"

# ── 2. Refusals keep their shape in BOTH builds ───────────────────────────
for build in on.s off.s; do
    expect keep_and_ne 9 "$build"
    expect keep_or_eq 9 "$build"
    expect keep_xor 9 "$build"
    expect keep_three_two 5 "$build"
    expect keep_one_two 5 "$build"
    expect keep_shared 9 "$build"
done

# ── 3. The kill switch restores the select spelling, shape for shape ───────
expect from_cmp 5 off.s
expect from_cmp_neg 5 off.s
expect from_lt 5 off.s
expect from_lt_neg 5 off.s
expect zero_or_one 5 off.s
expect one_or_zero 5 off.s
expect mask_of_sign 5 off.s
expect zero_or_one64 5 off.s
expect or_or 9 off.s
expect and_eq 9 off.s
expect or3 13 off.s
# The kill switch brings back the select arms for the shapes that had a
# select; the merges bring back their two separate selectors instead.
for label in from_cmp from_cmp_neg from_lt from_lt_neg zero_or_one one_or_zero \
             mask_of_sign zero_or_one64; do
    want off.s "$label" 'cmov'
done
# The merge is what halves the selectors: one `setcc` in the folded build,
# one per comparison in the knob-off build.
setne_count() { # $1 = build, $2 = label
    grep -Ec 'set(ne|e)[[:space:]]' <<<"$(body "$tmp/$1" "$2")" || true
}
[ "$(setne_count on.s or_or)" -eq 1 ] || { echo "or_or must have one selector in on.s" >&2; exit 1; }
[ "$(setne_count off.s or_or)" -eq 2 ] || { echo "or_or must keep two selectors in off.s" >&2; exit 1; }
[ "$(setne_count on.s and_eq)" -eq 1 ] || { echo "and_eq must have one selector in on.s" >&2; exit 1; }
[ "$(setne_count off.s and_eq)" -eq 2 ] || { echo "and_eq must keep two selectors in off.s" >&2; exit 1; }

# ── 4. The other backends never get longer ────────────────────────────────
cross_length_check() { # $1 = binary, $2 = target name
    local bin=$1 name=$2 after before labels label a b
    before="$tmp/$name-off.s"
    after="$tmp/$name-on.s"
    CCC_NO_PREDICATE_MATERIALIZE=1 "$bin" -O2 -S "$tmp/shapes.c" -o "$before"
    "$bin" -O2 -S "$tmp/shapes.c" -o "$after"
    labels=$(grep -oE '^[a-z_][a-zA-Z0-9_]*:$' "$before" | tr -d ':' | sort -u)
    for label in $labels; do
        a=$(count "$after" "$label")
        b=$(count "$before" "$label")
        if [ "$a" -gt "$b" ]; then
            echo "$name: $label got longer ($b -> $a):" >&2
            body "$after" "$label" >&2
            exit 1
        fi
    done
}
if [ -x "$bindir/lccc-i686" ]; then
    cross_length_check "$bindir/lccc-i686" i686
fi
if [ -x "$bindir/lccc-arm" ]; then
    cross_length_check "$bindir/lccc-arm" arm
fi
if [ -x "$bindir/lccc-riscv" ]; then
    cross_length_check "$bindir/lccc-riscv" riscv
fi

# ── 5. Answers: every level, both builds, and GCC agree ───────────────────
cat > "$tmp/exec.c" <<'C'
#include <stdio.h>
#include <stdint.h>
typedef unsigned u32;
typedef unsigned long u64;

static int from_cmp(int a, int b)     { return a != b ? 1 : 0; }
static int from_cmp_neg(int a, int b) { return a != b ? 0 : 1; }
static int from_lt(int a, int b)      { return a < b ? 1 : 0; }
static int from_lt_neg(int a, int b)  { return a < b ? 0 : 1; }
static u32 zero_or_one(int c)         { return c ? 1u : 0u; }
static int one_or_zero(int c)         { return c ? 0 : 1; }
static u32 mask_of_sign(int x)        { return x < 0 ? 0u : 1u; }
static u64 zero_or_one64(long long c) { return c ? 1ull : 0ull; }
static int or_or(int a, int b)        { return (a != 0) | (b != 0); }
static int and_eq(int a, int b)       { return (a == 0) & (b == 0); }
static int keep_and_ne(int a, int b)  { return (a != 0) & (b != 0); }
static int keep_or_eq(int a, int b)   { return (a == 0) | (b == 0); }
static int keep_xor(int a, int b)     { return (a != 0) ^ (b != 0); }

static unsigned h = 2166136261u;
static void mix(uint64_t v) {
    for (int i = 0; i < 8; i++) h = (h ^ (unsigned)((v >> (8 * i)) & 0xFFu)) * 16777619u;
}

int main(void) {
    static const int pat[] = {0, 1, -1, 2, -2, 0x7fffffff, (int)0x80000000u, 12345};
    const unsigned np = sizeof pat / sizeof pat[0];
    for (unsigned i = 0; i < np; i++) {
        for (unsigned j = 0; j < np; j++) {
            int a = pat[i], b = pat[j];
            mix((uint64_t)from_cmp(a, b));
            mix((uint64_t)from_cmp_neg(a, b));
            mix((uint64_t)from_lt(a, b));
            mix((uint64_t)from_lt_neg(a, b));
            mix((uint64_t)or_or(a, b));
            mix((uint64_t)and_eq(a, b));
            mix((uint64_t)keep_and_ne(a, b));
            mix((uint64_t)keep_or_eq(a, b));
            mix((uint64_t)keep_xor(a, b));
        }
        mix((uint64_t)zero_or_one(pat[i]));
        mix((uint64_t)one_or_zero(pat[i]));
        mix((uint64_t)mask_of_sign(pat[i]));
        mix(zero_or_one64((long long)pat[i] * 4));
    }
    printf("%08x\n", h);
    return 0;
}
C
sum=""
for level in -O0 -O1 -O2 -O3 -Os; do
    "$CCC" "$level" "$tmp/exec.c" -o "$tmp/ex$level"
    got=$("$tmp/ex$level")
    [ -z "$sum" ] || [ "$got" = "$sum" ] || {
        echo "$level checksum $got differs from $sum" >&2
        exit 1
    }
    sum=$got
done
CCC_NO_PREDICATE_MATERIALIZE=1 "$CCC" -O2 "$tmp/exec.c" -o "$tmp/exoff"
got=$("$tmp/exoff")
[ "$got" = "$sum" ] || { echo "CCC_NO_PREDICATE_MATERIALIZE=1 checksum $got differs from $sum" >&2; exit 1; }
if command -v gcc >/dev/null 2>&1; then
    gcc -O2 "$tmp/exec.c" -o "$tmp/exgcc"
    got=$("$tmp/exgcc")
    [ "$got" = "$sum" ] || { echo "gcc checksum $got differs from $sum" >&2; exit 1; }
fi

# ── 6. The vectorizer's mask store stays 0/1, not 0/-1 ────────────────────
# This is the shape the fold unlocks: with the select gone, the classifier's
# comparison is what the map vectorizer stores, and the packed compare is a
# lane MASK.  Every trip count around the vector width is checked.
cat > "$tmp/mask_store.c" <<'C'
#include <stdio.h>
#include <string.h>
#define N 1033
static int src[N], dst[N], ref[N];

void classify(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = (c >= 97 && c <= 122) ? 1 : 0;
    }
}
int main(void) {
    unsigned st = 20261101u;
    for (int i = 0; i < N; ++i) {
        st = st * 1664525u + 1013904223u;
        src[i] = (int)(st >> 7) % 200;
    }
    src[0] = 97; src[1] = 122; src[2] = 96; src[3] = 123; src[4] = -1;
    int fails = 0;
    for (unsigned long n = 0; n <= 72; ++n) {
        for (unsigned long i = 0; i < N; ++i) {
            volatile int vc = src[i];
            int c = vc;
            ref[i] = (c >= 97 && c <= 122) ? 1 : 0;
        }
        memset(dst, 0xAB, sizeof dst);
        classify(dst, src, n);
        if (memcmp(dst, ref, n * sizeof(int)) != 0) {
            printf("FAIL trip %lu\n", n);
            fails++;
        }
    }
    printf("%s\n", fails ? "VALIDATION FAILED" : "VALIDATION OK");
    return fails ? 1 : 0;
}
C
for level in -O1 -O2 -O3; do
    "$CCC" "$level" "$tmp/mask_store.c" -o "$tmp/ms$level"
    out=$("$tmp/ms$level")
    [ "$out" = "VALIDATION OK" ] || { echo "mask store at $level: $out" >&2; exit 1; }
done

echo "OK predicate_materialize ($sum)"
