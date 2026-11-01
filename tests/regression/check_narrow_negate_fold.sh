#!/usr/bin/env bash
# The narrow negate: `0 - x` is a `Neg`, and a 64-bit negate truncated to 32
# bits is the same negate at 32.
#
# Both rules come from the C integer promotions, which wrap narrow negations in
# a widen/negate/narrow round trip:
#
#   `0 - x`                lowers to   sub(0, x)          -> one `negl` once folded
#   `-(x != 0)`            lowers to   trunc32(neg64(sext64(cmp)))
#
# Left alone, the codegen cannot see through the second round trip and pays it
# in instructions it cannot remove: `setne; movzbl; movslq; negq; movl` where
# `testl; setne; negl` is the whole answer.  Negation is a ring operation and
# the extension's high bits die in the truncation, so `(I32)(-(I64)(I32)v)` is
# `-(I32)v` and both the extension and the second copy disappear.
#
# The fold is restricted to that one width pair because it MEASURED as a pure
# win there and as a pessimisation elsewhere (see the comment on the fold in
# `src/passes/simplify.rs`).  The narrower shapes are therefore pinned as
# controls: their counts must not move, so widening the fold later has to come
# with a fresh measurement instead of slipping in.
#
# The gate pins five things, because an instruction-count win that changes
# answers is worthless:
#
#   1. the emitted shapes (no extension/truncation round trip survives),
#   2. that the round trip comes back under `CCC_SIMPLIFY_SKIP=negcast`, which
#      is what proves the assertions above belong to this fold,
#   3. that `0 - x` is a `Neg` and the float spelling is NOT (IEEE: `0.0 - x`
#      is `+0.0` at `x = +0.0`, while `-x` is `-0.0`),
#   4. that a WIDENING cast keeps its wide negate (the high bits are observed),
#   5. that -O0, -O1, -O2, -O3 and both kill switches agree bit for bit -- on
#      every 8- and 16-bit input, and on float edge cases (-0.0, NaN payloads)
#      for the `zeroneg` control.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
tmp=${TMPDIR:-/tmp}/lccc-narrow-negate.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
unsigned short neg16(unsigned short x) { return (unsigned short)-x; }
unsigned char  neg8(unsigned char x)  { return (unsigned char)-x; }
short          negs16(short x)        { return (short)-x; }
unsigned short isol16(unsigned short x) { return x & (unsigned short)(0 - x); }
int            notzero(int x)         { return -(x != 0); }
/* Control: the negation is observed at the WIDE width, so it must stay there
   (folding it to `-(int)` would drop the sign extension of `x`). */
long           wide(int x)            { return -(long)x; }
/* Control: IEEE 754 says `0.0 - x` is not `-x` at `x = +0.0`, so the float
   subtraction must keep its `subsd` rather than become a sign flip. */
double         fsub(double x)         { return 0.0 - x; }
C

"$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes.s"
CCC_SIMPLIFY_SKIP=negcast "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/no_negcast.s"
CCC_SIMPLIFY_SKIP=zeroneg "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/no_zeroneg.s"

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

# ── 1. The folded shape: a 64-bit negate truncated to 32 bits ──────────────
n=$(count "$tmp/shapes.s" notzero)
[ "$n" -le 6 ] || {
    echo "notzero costs $n instructions; the I64 round trip is back:" >&2
    fn_body "$tmp/shapes.s" notzero >&2
    exit 1
}
lacks "$(fn_body "$tmp/shapes.s" notzero)" 'movslq\|negq' "notzero negates at 64 bits"
if ! has "$(fn_body "$tmp/shapes.s" notzero)" 'negl'; then
    echo "notzero must negate at 32 bits:" >&2
    fn_body "$tmp/shapes.s" notzero >&2
    exit 1
fi

# ── 1b. Controls: the narrower shapes the fold deliberately does not touch ──
# Their counts are pinned exactly.  A shape that got shorter here means the
# fold was widened without the measurement that narrowed it in the first place
# (+60 on `packed_mask_shifts.c`, +3 and +1 from spills); a shape that got
# longer is a regression either way.
expect() { # $1 = label, $2 = expected count, $3 = build
    local n
    n=$(count "$tmp/$3" "$1")
    if [ "$n" -ne "$2" ]; then
        echo "$1 costs $n instructions in $3, expected $2:" >&2
        fn_body "$tmp/$3" "$1" >&2
        exit 1
    fi
}
expect neg16 4 shapes.s
expect neg8 4 shapes.s
expect negs16 4 shapes.s
expect isol16 6 shapes.s
expect neg16 4 no_negcast.s
expect neg8 4 no_negcast.s
expect negs16 4 no_negcast.s
expect isol16 6 no_negcast.s

# ── 2. The fold owns the shapes ─────────────────────────────────────────────
# With `negcast` off the round trip must return, otherwise the assertions above
# are satisfied by some other rule and would not catch this one regressing.
n=$(count "$tmp/no_negcast.s" notzero)
[ "$n" -gt 6 ] || {
    echo "CCC_SIMPLIFY_SKIP=negcast leaves notzero folded ($n instructions)" >&2
    fn_body "$tmp/no_negcast.s" notzero >&2
    exit 1
}
if ! has "$(fn_body "$tmp/no_negcast.s" notzero)" 'negq'; then
    echo "CCC_SIMPLIFY_SKIP=negcast does not restore the 64-bit negate:" >&2
    fn_body "$tmp/no_negcast.s" notzero >&2
    exit 1
fi

# ── 3. `0 - x` is a negate, `0.0 - x` is not ────────────────────────────────
body=$(fn_body "$tmp/shapes.s" fsub)
if ! has "$body" 'subsd'; then
    echo "fsub lost its float subtraction (0.0 - x is not -x at +0.0):" >&2
    echo "$body" >&2
    exit 1
fi
# A float negation is a sign-bit XOR against a materialized mask; the
# subtraction itself legitimately self-XORs to build the 0.0 operand.
lacks "$body" 'xorpd[^#]*\.L\|xorpd[^#]*%rip' "fsub replaced the subtraction with a sign flip"
body=$(fn_body "$tmp/no_zeroneg.s" isol16)
if ! has "$body" 'sub[lq] '; then
    echo "control isol16 with zeroneg off does not restore the subtraction:" >&2
    echo "$body" >&2
    exit 1
fi
# That control is one instruction SHORTER (5 against 6), and that is stated
# here rather than hidden: `zeroneg` leaves a 32-bit `Neg` whose 16-bit
# consumers must mask (`negl; movzwl`), while the subtraction stayed wide and
# folded into the 64-bit `andq`.  The right fix is a narrow-ALU rule (an `And`
# at the narrow width should not need both operands masked), which is a
# separate, measured change; the `zeroneg` win it belongs to is -439 on
# `packed_mask_shifts.c`, so trading one site against that is not close.
expect isol16 5 no_zeroneg.s

# ── 4. A widening cast keeps its wide negate ────────────────────────────────
# `-(long)x` observes 64 bits of the negate, so the sign extension must stay in
# front of it.  Folding here would be a miscompile, and the shape is one
# `negcast` step away from the ones above.
body=$(fn_body "$tmp/shapes.s" wide)
if ! has "$body" 'movslq\|movsxd'; then
    echo "wide lost the sign extension of its 64-bit negation:" >&2
    echo "$body" >&2
    exit 1
fi
if ! has "$body" 'negq'; then
    echo "wide must negate at 64 bits:" >&2
    echo "$body" >&2
    exit 1
fi

# ── 5. Execution: every build agrees bit for bit ────────────────────────────
cat > "$tmp/exec.c" <<'C'
#include <stdio.h>
#include <string.h>

static unsigned short neg16(unsigned short x) { return (unsigned short)-x; }
static unsigned char  neg8(unsigned char x)  { return (unsigned char)-x; }
static short          negs16(short x)        { return (short)-x; }
static unsigned short isol16(unsigned short x) { return x & (unsigned short)(0 - x); }
static int            notzero(int x)         { return -(x != 0); }
static long           wide(int x)            { return -(long)x; }
static double         fsub(double x)         { return 0.0 - x; }
/* The result is observed as `unsigned`, so the zero extension of the narrow
   negation is part of the answer: this row is what a fold that trusts the low
   bits alone would break. */
static unsigned       keep_width(unsigned short x) { return (unsigned short)-x; }

static unsigned long long fnv = 1469598103934665603ull;
static void mix(unsigned long long v) {
    for (int i = 0; i < 8; i++) {
        fnv ^= (v >> (8 * i)) & 0xffu;
        fnv *= 1099511628211ull;
    }
}

int main(void) {
    for (unsigned x = 0; x <= 0xffffu; x++) {
        unsigned short s = (unsigned short)x;
        mix(neg16(s));
        mix(neg8((unsigned char)x));
        mix((unsigned short)negs16((short)s));
        mix(isol16(s));
        mix(keep_width(s));
        mix((unsigned)wide((int)s));
        mix(notzero((int)s));
        mix(notzero(-(int)s));
        mix(notzero(0));
        unsigned bits;
        double d = (double)(int)s * 0.5;
        memcpy(&bits, &(double){fsub(d)}, sizeof bits);
        mix(bits);
    }
    /* Float edge cases: +0.0 (the case that makes `0.0 - x` differ from
       `-x`), -0.0, infinities and a NaN payload. */
    {
        double specials[] = {0.0, -0.0, 1.0, -1.0, 1e308 * 10, -1e308 * 10, 0.0 / 0.0};
        for (unsigned i = 0; i < sizeof specials / sizeof specials[0]; i++) {
            unsigned bits;
            memcpy(&bits, &(double){fsub(specials[i])}, sizeof bits);
            mix(bits);
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
        echo "-O0 and $opt disagree on the narrow-negate answers:" >&2
        diff "$ref" "$tmp/out$opt" | sed -n '1,10p' >&2
        exit 1
    fi
done

# The kill switches may change the code, never the answers.  (The float row is
# in the same program on purpose: the `zeroneg` guard is what keeps `0.0 - x`
# a subtraction, and the answers must be bit-identical either way.)
for knob in CCC_SIMPLIFY_SKIP=negcast CCC_SIMPLIFY_SKIP=zeroneg; do
    env $knob "$CCC" -O2 "$tmp/exec.c" -o "$tmp/ek"
    "$tmp/ek" > "$tmp/outk"
    if ! cmp -s "$ref" "$tmp/outk"; then
        echo "$knob changes the answers:" >&2
        diff "$ref" "$tmp/outk" | sed -n '1,10p' >&2
        exit 1
    fi
done

# The oracle: GCC must reach the same answers through the source expression.
if command -v "${GCC_ORACLE:-gcc-16}" >/dev/null 2>&1; then
    "${GCC_ORACLE:-gcc-16}" -O2 "$tmp/exec.c" -o "$tmp/egcc"
    "$tmp/egcc" > "$tmp/outgcc"
    if ! cmp -s "$ref" "$tmp/outgcc"; then
        echo "lccc and GCC disagree on the narrow-negate answers:" >&2
        diff "$ref" "$tmp/outgcc" | sed -n '1,10p' >&2
        exit 1
    fi
fi

# ── Instruction counts, printed so a drift is visible in the CI log ─────────
for name in neg16 neg8 negs16 isol16 notzero wide fsub; do
    echo "  $name: $(count "$tmp/shapes.s" "$name") instructions (negcast off: $(count "$tmp/no_negcast.s" "$name"))"
done

echo "narrow-negate gate OK"
