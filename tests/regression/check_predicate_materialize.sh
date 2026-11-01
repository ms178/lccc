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
#   4. the other backends never get *longer* (i686, AArch64, RISC-V), and the
#      indicator fold of section 7 fires on all of them,
#   5. -O0, -O1, -O2, -O3, -Os, the kill switch and GCC agree bit for bit on a
#      checksum over the shapes,
#   6. the indicator fold refuses what it cannot prove (different operand pairs,
#      mixed signedness classes), and every answer it produces agrees with a
#      branch-written reference over 231,000 comparisons,
#   8. the vectorizer's mask store stays correct: a loop that stores
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

# ── 7. A three-way comparator folds back to the compare it came from ───────
# `(a > b) - (a < b)` is the sign of `a - b`, and comparing it against zero is
# comparing `a` and `b`.  Once `c ? 1 : 0` is materialized as a comparison, the
# sign value is what an inlined comparator (`if (key_cmp(x, y) <= 0)`) hands
# to its caller -- and a fold that only knew the `Select` spelling left the
# two indicators, the subtraction and a second comparison behind (nine
# instructions where one `cmp` is the answer).  The entry point is the same
# question in `simplify`, so the same gate owns it: the folded shape, its
# predicate table, and the refusals that must not move.
cat > "$tmp/three_way.c" <<'C'
/* A zero-extension is non-negative, so the SIGNED question against zero
   answers itself: `(int)(unsigned char)a < 0` is false for every input, and
   `<= 0`/`> 0` are `== 0`/`!= 0` at the narrow width.  GCC and Clang fold both
   spellings; LCCC knew only the unsigned-typed operand. */
int zx_lt0(int a) { return (int)(unsigned char)a < 0; }
int zx_le0(int a) { return (int)(unsigned char)a <= 0; }
int le(int a, int b)  { int t = (a > b) - (a < b); int u = t; return u <= 0; }
int lt(int a, int b)  { int t = (a > b) - (a < b); int u = t; return u < 0; }
int rev(int a, int b) { int t = (a < b) - (a > b); int u = t; return u <= 0; }
int byte(int a, int b){ int t = (a > b) - (a < b); int u = t; return (unsigned)u > 0u; }
/* A NESTED pair: the two indicators are not complementary but nested, so their
   difference is the equality indicator -- `(a >= b) - (a > b)` is 1 exactly when
   `a == b` -- and `u > 0` is `a == b`.  The same enumeration answers it. */
int nested_eq(int a, int b) { int t = (a >= b) - (a > b); int u = t; return u > 0; }
/* Refusals.  Different operand pairs relate nothing. */
int keep_pairs(int a, int b, int c, int d) { int t = (a > b) - (c < d); int u = t; return u <= 0; }
/* Mixed signedness classes are not complementary either: `a = -1, b = 1` is
   false for both `a >s b` and `a <u b`, so the difference is not the sign of
   anything and the shape must stay. */
int keep_mixed(int a, int b) { int t = (a > b) - (int)((unsigned)a < (unsigned)b); int u = t; return u <= 0; }
/* A second reader of the sign keeps the subtraction alive; the folded compare
   may not cost more than the unfolded one. */
int g_sink;
int keep_second_reader(int a, int b) { int t = (a > b) - (a < b); g_sink = t; int u = t; return u <= 0; }
C
"$CCC" -O2 -S "$tmp/three_way.c" -o "$tmp/three_way_on.s"
CCC_SIMPLIFY_SKIP=booldiff "$CCC" -O2 -S "$tmp/three_way.c" -o "$tmp/three_way_off.s"
count_in() { # $1 = asm file, $2 = label
    fn_body "$1" "$2" | grep -c -e '^[[:space:]]\+[a-z]' || true
}
for label in le lt rev byte nested_eq; do
    on=$(count_in "$tmp/three_way_on.s" "$label")
    off=$(count_in "$tmp/three_way_off.s" "$label")
    [ "$on" -le 5 ] || {
        printf '%s: the folded three-way comparator costs %s instructions:\n' "$label" "$on" >&2
        fn_body "$tmp/three_way_on.s" "$label" >&2
        exit 1
    }
    [ "$off" -gt "$on" ] || {
        printf '%s: the fold has no effect (%s on, %s with CCC_SIMPLIFY_SKIP=booldiff)\n' \
            "$label" "$on" "$off" >&2
        exit 1
    }
    # One compare and one `setcc` -- the answer's own.  Two of each means the
    # two indicators survived, which is exactly the shape this fold removes.
    body=$(fn_body "$tmp/three_way_on.s" "$label")
    cmps=$(grep -cE '^[[:space:]]+cmp' <<<"$body" || true)
    sets=$(grep -cE '^[[:space:]]+set[a-z]+ ' <<<"$body" || true)
    if [ "$cmps" -ne 1 ] || [ "$sets" -ne 1 ]; then
        printf '%s: %s compares and %s setccs, expected one of each:\n' \
            "$label" "$cmps" "$sets" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
    if grep -qE '^[[:space:]]+sub[lqwb] ' <<<"$body"; then
        printf '%s: the sign subtraction survived:\n' "$label" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
done
# The refusals keep their counts in BOTH builds, which is what proves the
# assertions above belong to the indicator fold and not to a shape that was
# going to collapse anyway.
for label in keep_pairs keep_mixed; do
    on=$(count_in "$tmp/three_way_on.s" "$label")
    off=$(count_in "$tmp/three_way_off.s" "$label")
    [ "$on" -eq "$off" ] || {
        printf '%s moved without the fold (%s on, %s off); the refusal leaks:\n' \
            "$label" "$on" "$off" >&2
        exit 1
    }
done
# A second reader of the sign cannot make the comparison longer: the fold
# replaces one compare with one compare and only removes the second consumer.
on=$(count_in "$tmp/three_way_on.s" keep_second_reader)
off=$(count_in "$tmp/three_way_off.s" keep_second_reader)
[ "$on" -le "$off" ] || {
    printf 'keep_second_reader got longer (%s on, %s off)\n' "$on" "$off" >&2
    exit 1
}

# The known-non-negative rule is `simplify_cmp`'s, not the indicator fold's, so
# it is attributed through `CCC_SIMPLIFY_SKIP=cmp`: `zx_lt0` is the constant `0`
# (two instructions including `ret`) with the rule and a real comparison without
# it, while `zx_le0` is count-neutral -- `Eq 0` against a byte and `Ule 0` both
# select a `testb` plus one `setcc`, so only the predicate moves (`sete` instead
# of `setbe`) and the assertion pins that it stays at four.
CCC_SIMPLIFY_SKIP=cmp "$CCC" -O2 -S "$tmp/three_way.c" -o "$tmp/three_way_nocmp.s"
for pair in "zx_lt0 2 4" "zx_le0 4 4"; do
    set -- $pair
    on=$(count_in "$tmp/three_way_on.s" "$1")
    off=$(count_in "$tmp/three_way_nocmp.s" "$1")
    [ "$on" -le "$2" ] || {
        printf '%s: expected at most %s instructions, got %s:\n' "$1" "$2" "$on" >&2
        fn_body "$tmp/three_way_on.s" "$1" >&2
        exit 1
    }
    [ "$off" -ge "$3" ] || {
        printf '%s: CCC_SIMPLIFY_SKIP=cmp did not restore the comparison (%s):\n' "$1" "$off" >&2
        fn_body "$tmp/three_way_nocmp.s" "$1" >&2
        exit 1
    }
done

# ── 7b. The indicator fold on every backend ───────────────────────────────
# The fold is IR-level, so every target must gain from it and none may lose:
# same probe, `booldiff` skipped as the only difference.  This is also the
# cross-target proof for the three-way shape, since the corpus sweep cannot
# compile the valence test for these targets (no cross libc in CI's sandbox).
cross_three_way() { # $1 = binary, $2 = target name
    local bin=$1 name=$2 label on off
    "$bin" -O2 -S "$tmp/three_way.c" -o "$tmp/tw-$name-on.s"
    CCC_SIMPLIFY_SKIP=booldiff "$bin" -O2 -S "$tmp/three_way.c" -o "$tmp/tw-$name-off.s"
    for label in le lt rev byte nested_eq; do
        on=$(count_in "$tmp/tw-$name-on.s" "$label")
        off=$(count_in "$tmp/tw-$name-off.s" "$label")
        [ "$on" -le "$off" ] || {
            printf '%s: %s got longer with the fold (%s on, %s off)\n' \
                "$name" "$label" "$on" "$off" >&2
            fn_body "$tmp/tw-$name-on.s" "$label" >&2
            exit 1
        }
    done
    # And it must actually fire there: `le` is two `setcc`s and a `sub` wide.
    on=$(count_in "$tmp/tw-$name-on.s" le)
    off=$(count_in "$tmp/tw-$name-off.s" le)
    [ "$off" -gt "$on" ] || {
        printf '%s: the fold does not fire on this target (%s on, %s off)\n' \
            "$name" "$on" "$off" >&2
        exit 1
    }
}
for t in i686 arm riscv; do
    if [ -x "$bindir/lccc-$t" ]; then
        cross_three_way "$bindir/lccc-$t" "$t"
    fi
done

# ── 8. The values, not the counts ──────────────────────────────────────────
# Every shape above is a question about `a - b`, and a fold that deletes six
# instructions is worth nothing if it changes an answer.  This is the oracle:
# `three_way_boolean_compare.c` checks every predicate over every width and
# signedness against a branch-written reference, on the extrema, the values
# around zero, 40,000 seeded pairs and a full +-128 x +-128 sweep, and it also
# covers the shapes a *generalized* rule must refuse (mixed classes, different
# pairs, a difference widened through an `unsigned char`), which no count can
# distinguish from a correct fold.
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
"$CCC" -O2 "$here/three_way_boolean_compare.c" -o "$tmp/three_way_values"
out=$("$tmp/three_way_values")
[ "$out" = "VALIDATION OK" ] || {
    printf 'three_way_boolean_compare: %s\n' "$out" >&2
    exit 1
}

echo "OK predicate_materialize ($sum)"
