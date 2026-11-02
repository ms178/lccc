#!/usr/bin/env bash
# Comparison dominance: a conjunction of two comparisons of ONE value is its
# stronger conjunct, and a disjunction is its weaker one.
#
#   (x != 0) & (x > 4)      is   x > 4       the floor already excludes zero
#   (x > 0)  & (x > 99)     is   x > 99      the tighter floor decides both
#   (x < 10) & (x < 5)      is   x < 5       the tighter ceiling decides both
#   (x != 0) | (x > 4)      is   x != 0      the floor implies the guard
#
# C spells range guards this way everywhere (`if (n && n > 4)`,
# `p && p->len > 8` after inlining, `assert(i < n && i < max)`), and each site
# costs five instructions when the redundant comparison survives: a second
# compare, its materialization, the logic op, and the move that joins them.
# Both oracles fold it; LCCC emitted `setne; movzbl; setg; movzbl; andl` (9)
# where `cmp $4; setg` (4) is the whole answer.
#
# The gate pins six things, because an instruction-count win that changes
# answers is worthless:
#
#   1. every dominant shape folds, at 8, 16, 32 and 64 bits, in a conjunction
#      and (keeping the implied operand) in a disjunction,
#   2. the folded body contains the surviving comparison's flag only -- the
#      eliminated comparison's `set*` must be gone,
#   3. the two shapes that must NOT fold stay at their exact count in both
#      builds: comparisons of DIFFERENT values, and two bounds that describe
#      nothing while neither implies the other,
#   4. `CCC_NO_COMPARE_DOMINANCE=1` brings every folded shape back, which is
#      what proves the assertions above belong to this fold and not to some
#      other rule that happens to agree today,
#   5. -O0, -O1, -O2, -O3 and the kill switch agree bit for bit on a checksum
#      over every 16-bit input, both sign edges, and 64-bit operands,
#   6. GCC reaches the same checksum from the same source.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
tmp=${TMPDIR:-/tmp}/lccc-compare-dominance.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
/* Implied non-zero: the signed floor already excludes zero. */
int      guard(int x)             { return (x != 0) & (x > 4); }
/* Same op, two floors: the tighter one is the answer. */
int      floor_(int x)            { return (x > 0) & (x > 99); }
/* Same op, two ceilings, unsigned. */
unsigned ceiling(unsigned x)      { return (x < 10) & (x < 5); }
/* Implied non-zero across the unsigned ordering. */
unsigned zero_floor(unsigned x)   { return (x != 0) & (x >= 7); }
/* The disjunction keeps the WEAKER operand: the guard, not the floor. */
int      or_guard(int x)          { return (x != 0) | (x > 4); }
/* Widths: the fold is a `Copy` of one operand, so no width is special. */
long     guard64(long x)          { return (x != 0) & (x > 4); }
short    guard16(short x)         { return (x != 0) & (x > 4); }
signed char guard8(signed char x) { return (x != 0) & (x > 4); }

/* Controls.  Two bounds that describe nothing: neither implies the other, so
   the pair must stay as written (the range folder owns emptiness). */
int      unrelated(int x)         { return (x < 5) & (x > 10); }
/* Control.  Two DIFFERENT values: neither comparison says anything about the
   other. */
int      two_values(int x, int y) { return (x != 0) & (y > 4); }
C

"$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes.s"
CCC_NO_COMPARE_DOMINANCE=1 "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/off.s"

fn_body() { # $1 = asm file, $2 = label; body between the label and ret
    sed -n "/^$2:/,/ret/p" "$1"
}
count() { # $1 = asm file, $2 = label
    fn_body "$1" "$2" | grep -c -e '^[[:space:]]\+[a-z]' || true
}
has() {
    grep -q -e "$2" <<<"$1"
}
lacks() { # $1 = text, $2 = pattern, $3 = message
    if grep -q -e "$2" <<<"$1"; then
        echo "$3: unexpected '$2' in:" >&2
        echo "$1" >&2
        return 1
    fi
}
expect() { # $1 = label, $2 = expected count, $3 = build
    local n
    n=$(count "$tmp/$3" "$1")
    if [ "$n" -ne "$2" ]; then
        echo "$1 costs $n instructions in $3, expected $2:" >&2
        fn_body "$tmp/$3" "$1" >&2
        exit 1
    fi
}

# ── 1. Every dominant shape folds, and the eliminated flag is gone ─────────
expect guard 4 shapes.s
expect floor_ 4 shapes.s
expect ceiling 4 shapes.s
expect zero_floor 4 shapes.s
expect guard64 5 shapes.s
expect guard16 5 shapes.s
expect guard8 4 shapes.s

for name in guard floor_ zero_floor guard64 guard16 guard8; do
    lacks "$(fn_body "$tmp/shapes.s" "$name")" 'setne' \
        "$name kept the redundant non-zero test"
done
if ! has "$(fn_body "$tmp/shapes.s" guard)" 'setg'; then
    echo "guard must keep the floor's `setg`:" >&2
    fn_body "$tmp/shapes.s" guard >&2
    exit 1
fi
if ! has "$(fn_body "$tmp/shapes.s" ceiling)" 'setb'; then
    echo "ceiling must keep the tighter ceiling's `setb`:" >&2
    fn_body "$tmp/shapes.s" ceiling >&2
    exit 1
fi
if ! has "$(fn_body "$tmp/shapes.s" zero_floor)" 'setae'; then
    echo "zero_floor must keep the unsigned floor's `setae`:" >&2
    fn_body "$tmp/shapes.s" zero_floor >&2
    exit 1
fi

# ── 2. The disjunction keeps the weaker operand ────────────────────────────
expect or_guard 4 shapes.s
if ! has "$(fn_body "$tmp/shapes.s" or_guard)" 'setne'; then
    echo "or_guard must answer with the guard:" >&2
    fn_body "$tmp/shapes.s" or_guard >&2
    exit 1
fi
lacks "$(fn_body "$tmp/shapes.s" or_guard)" 'cmpl[[:space:]]\+\$4' \
    "or_guard answered with the floor instead of the guard"

# ── 3. Controls: the shapes that must not fold ─────────────────────────────
expect unrelated 9 shapes.s
expect unrelated 9 off.s
expect two_values 9 shapes.s
expect two_values 9 off.s

# ── 4. The kill switch owns the shapes ─────────────────────────────────────
# With the fold off every folded shape must be back at its unfolded count and
# the non-zero test must be back, otherwise the assertions above hold for some
# other reason and would not catch this fold regressing.
for name in guard floor_ ceiling zero_floor or_guard guard64 guard16 guard8; do
    expect "$name" 9 off.s
done
if ! has "$(fn_body "$tmp/off.s" guard)" 'setne'; then
    echo "CCC_NO_COMPARE_DOMINANCE=1 does not restore the non-zero test:" >&2
    fn_body "$tmp/off.s" guard >&2
    exit 1
fi

# ── 5. Execution: every build agrees bit for bit ───────────────────────────
cat > "$tmp/exec.c" <<'C'
#include <stdio.h>
#include <limits.h>

static int      guard(int x)             { return (x != 0) & (x > 4); }
static int      floor_(int x)            { return (x > 0) & (x > 99); }
static unsigned ceiling(unsigned x)      { return (x < 10) & (x < 5); }
static unsigned zero_floor(unsigned x)   { return (x != 0) & (x >= 7); }
static int      or_guard(int x)          { return (x != 0) | (x > 4); }
static long     guard64(long x)          { return (x != 0) & (x > 4); }
static short    guard16(short x)         { return (x != 0) & (x > 4); }
static signed char guard8(signed char x) { return (x != 0) & (x > 4); }
static int      unrelated(int x)         { return (x < 5) & (x > 10); }
static int      two_values(int x, int y) { return (x != 0) & (y > 4); }
/* The same dominance one step away from the accepted shapes: a guard against a
   bound that does NOT exclude zero must not be dropped. */
static int      negative_floor(int x)    { return (x != 0) & (x > -1); }
/* ... and one where the guard IS redundant again, at the signed minimum. */
static int      min_floor(int x)         { return (x != 0) & (x > INT_MIN); }

static unsigned long long fnv = 1469598103934665603ull;
static void mix(unsigned long long v) {
    for (int i = 0; i < 8; i++) {
        fnv ^= (v >> (8 * i)) & 0xffu;
        fnv *= 1099511628211ull;
    }
}

int main(void) {
    for (unsigned u = 0; u <= 0xffffu; u++) {
        short s = (short)u;
        mix((unsigned)guard((int)s));
        mix((unsigned)floor_((int)s));
        mix(ceiling(u));
        mix(zero_floor(u));
        mix((unsigned)or_guard((int)s));
        mix((unsigned long long)guard64((long)s));
        mix((unsigned)guard16(s));
        mix((unsigned)(signed char)guard8((signed char)s));
        mix((unsigned)unrelated((int)s));
        mix((unsigned)two_values((int)s, (int)(unsigned short)(u * 7u)));
        mix((unsigned)negative_floor((int)s));
        mix((unsigned)min_floor((int)s));
    }
    /* The sign edges, where an ordering argument written at 32 bits would go
       wrong if it mixed signed and unsigned orderings. */
    {
        int edges[] = {INT_MIN, INT_MIN + 1, -100, -1, 0, 1, 4, 5, 99, 100, INT_MAX};
        for (unsigned i = 0; i < sizeof edges / sizeof edges[0]; i++) {
            for (unsigned j = 0; j < sizeof edges / sizeof edges[0]; j++) {
                mix((unsigned)guard(edges[i]));
                mix((unsigned)floor_(edges[i]));
                mix(ceiling((unsigned)edges[i]));
                mix(zero_floor((unsigned)edges[i]));
                mix((unsigned)or_guard(edges[i]));
                mix((unsigned long long)guard64((long)edges[i] * 1000003L));
                mix((unsigned)unrelated(edges[i]));
                mix((unsigned)two_values(edges[i], edges[j]));
                mix((unsigned)negative_floor(edges[i]));
                mix((unsigned)min_floor(edges[i]));
            }
        }
    }
    printf("%016llx\n", fnv);
    return 0;
}
C

ref=""
for opt in -O0 -O1 -O2 -O3; do
    "$CCC" "$opt" "$tmp/exec.c" -o "$tmp/e$opt"
    "$tmp/e$opt" > "$tmp/out$opt"
    if [ -z "$ref" ]; then
        ref="$tmp/out$opt"
    elif ! cmp -s "$ref" "$tmp/out$opt"; then
        echo "-O0 and $opt disagree on the dominance answers:" >&2
        diff "$ref" "$tmp/out$opt" | sed -n '1,10p' >&2
        exit 1
    fi
done

# The kill switch may change the code, never the answers.
CCC_NO_COMPARE_DOMINANCE=1 "$CCC" -O2 "$tmp/exec.c" -o "$tmp/ek"
"$tmp/ek" > "$tmp/outk"
if ! cmp -s "$ref" "$tmp/outk"; then
    echo "CCC_NO_COMPARE_DOMINANCE=1 changes the answers:" >&2
    diff "$ref" "$tmp/outk" | sed -n '1,10p' >&2
    exit 1
fi

# The oracle: GCC must reach the same answers through the source expression.
if command -v "${GCC_ORACLE:-gcc-16}" >/dev/null 2>&1; then
    "${GCC_ORACLE:-gcc-16}" -O2 "$tmp/exec.c" -o "$tmp/egcc"
    "$tmp/egcc" > "$tmp/outgcc"
    if ! cmp -s "$ref" "$tmp/outgcc"; then
        echo "lccc and GCC disagree on the dominance answers:" >&2
        diff "$ref" "$tmp/outgcc" | sed -n '1,10p' >&2
        exit 1
    fi
fi

# ── Instruction counts, printed so a drift is visible in the CI log ────────
for name in guard floor_ ceiling zero_floor or_guard guard64 guard16 guard8 unrelated two_values; do
    echo "  $name: $(count "$tmp/shapes.s" "$name") instructions (fold off: $(count "$tmp/off.s" "$name"))"
done

echo "compare-dominance gate OK"
