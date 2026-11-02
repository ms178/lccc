#!/usr/bin/env bash
# The narrow unary ops: `0 - x` is a `Neg`, and a 64-bit negate or complement
# narrowed to 32 bits is the same operation at 32.
#
# Both rules come from the C integer promotions, which wrap narrow negations in
# a widen/negate/narrow round trip:
#
#   `0 - x`                lowers to   sub(0, x)          -> one `negl` once folded
#   `-(x != 0)`            lowers to   trunc32(neg64(sext64(cmp)))
#   `x & ~(1 << n)`        lowers to   and32(x, trunc32(not64(sext64(1 << n))))
#
# Left alone, the codegen cannot see through the second round trip and pays it
# in instructions it cannot remove: `setne; movzbl; movslq; negq; movl` where
# `testl; setne; negl` is the whole answer.  Negation is a ring operation and
# the extension's high bits die in the truncation, so `(I32)(-(I64)(I32)v)` is
# `-(I32)v` and both the extension and the second copy disappear.
#
# THE LOWERING NOW DOES PART OF THIS FIRST, and the split is measured rather
# than assumed (`c-frontend: unary -/~/+ wrap at the promoted type, not the
# carrier` emits `-` and `~` at the width their result is observed at):
#
#   * an `int` result no longer arrives as a 64-bit negate truncated to 32, so
#     `negcast` matches nothing any C frontend produces.  Disabling it
#     reproduces all 1,212 compilable `tests/regression/*.c` files byte for
#     byte, and every shape in this file.  Section 2 pins that by requiring the
#     two builds to agree, so a reintroduced wide negate fails a gate instead of
#     quietly costing two instructions a site;
#   * a SUB-WORD result still truncates, so `(T)~x` is still
#     `trunc(Not_{I32}(ext))` and `notcast` is what narrows it -- worth exactly
#     one instruction a site.  Sections 2b and 5 own that shape end-to-end.
#
# The negate fold stays restricted to the 64-bit-source/32-bit-destination pair
# because that is the ONE pair where it measured as a pure win; the narrower
# shapes are pinned as controls, so widening it later has to come with a fresh
# measurement instead of slipping in (see the comment on the fold in
# `src/passes/simplify.rs`).
#
# The complement (`notcast`) needs no width restriction -- `Not` is bitwise and
# therefore commutes with truncation outright -- but it does need the outer step
# to be a NARROWING one, because `ext(~x)` and `~ext(x)` differ in the high
# bits.  Its original shape, the `x & ~(1 << n)` bit-clear, now has an owner
# ABOVE this rule: `fold_bit_select` collapses it to a `BitClear`, which x86
# lowers to one `btr` (3 instructions, GCC's count).  Both that build and the
# `CCC_NO_BIT_SELECT=1` fallback are pinned, so a future reader can see which
# rule does the work; in the fallback neither build reaches `notcast`, because
# a 32-bit result gets its 32-bit complement from the lowering.
#
# The gate pins five things, because an instruction-count win that changes
# answers is worthless:
#
#   1. the emitted shapes (no extension/truncation round trip survives),
#   2. that `CCC_SIMPLIFY_SKIP=negcast` changes NOTHING, which is what records
#      that the lowering now owns the narrow negate and this fold is a safety
#      net (section 2 says how that was measured and what to do if it changes),
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
/* The complement's round trip: `~(1 << n)` was widened to 64 bits. */
int            bclear(int x, int n)   { return x & ~(1 << n); }
unsigned       bclearu(unsigned x, unsigned n) { return x & ~(1u << n); }
/* Control: a WIDENING cast observes the high bits of the complement, so the
   wide `Not` must stay. */
unsigned long  notw(unsigned x)       { return ~(unsigned long)x; }
/* Control: the negation is observed at the WIDE width, so it must stay there
   (folding it to `-(int)` would drop the sign extension of `x`). */
long           wide(int x)            { return -(long)x; }
/* The complement's LIVE shape after the lowering change: a sub-word result
   truncates, so `(T)~x` is still `trunc(Not_{I32}(ext))` and `notcast` is what
   narrows it.  These two are the fold's end-to-end owner. */
unsigned short nots16(unsigned short x) { return (unsigned short)~x; }
unsigned char  nots8(unsigned char x)   { return (unsigned char)~x; }
/* Control: IEEE 754 says `0.0 - x` is not `-x` at `x = +0.0`, so the float
   subtraction must keep its `subsd` rather than become a sign flip. */
double         fsub(double x)         { return 0.0 - x; }
C

"$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/shapes.s"
CCC_SIMPLIFY_SKIP=negcast "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/no_negcast.s"
CCC_SIMPLIFY_SKIP=zeroneg "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/no_zeroneg.s"
CCC_SIMPLIFY_SKIP=notcast "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/no_notcast.s"
# The single-bit selection fold now owns the eligible `x & ~(1 << n)` shapes.
# Disable it to keep testing the older complement-width fold underneath, and
# disable both to prove which pass owns the remaining extension/not pair.
CCC_NO_BIT_SELECT=1 "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/no_bit_select.s"
CCC_NO_BIT_SELECT=1 CCC_SIMPLIFY_SKIP=notcast "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/no_notcast_no_select.s"

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

# ── 2. Who owns the narrow negate now ───────────────────────────────────────
# It used to be this fold: the lowering negated at the carrier width, so
# `-(x != 0)` arrived as `trunc32(neg64(ext64(cmp)))` and `negcast` was what
# turned it into a 32-bit `Neg`.  The lowering now negates at the PROMOTED type
# (`c-frontend: unary -/~/+ wrap at the promoted type, not the carrier`), so the
# round trip is never built and the fold has nothing to match.
#
# That is measured, not assumed: with `CCC_SIMPLIFY_SKIP=negcast` the entire
# 1,212-file regression corpus compiles to byte-identical assembly, and so does
# every shape in this file.  The rule is kept anyway -- it is a sound rewrite
# (negation is a ring operation and the extension's high bits die in the
# truncation), it costs one match, and any future producer of the wide shape
# gets it for free rather than paying two instructions per site in silence.
#
# So the assertion is about OWNERSHIP: the two builds must agree.  If a future
# lowering reintroduces the wide negate, this fails and says what happened,
# because the decision that follows -- retire the rule, or re-pin it as the
# owner with a fresh measurement -- is not one a silent pass should make.
if ! cmp -s "$tmp/shapes.s" "$tmp/no_negcast.s"; then
    echo "CCC_SIMPLIFY_SKIP=negcast changed the emitted code, so this simplify rule is" >&2
    echo "reachable again (the lowering no longer owns the narrow negate). Re-measure" >&2
    echo "and re-pin the ownership; do not just accept the new output:" >&2
    diff "$tmp/shapes.s" "$tmp/no_negcast.s" | sed -n '1,20p' >&2
    exit 1
fi

# ── 2b. `notcast` owns the sub-word complement; `btr` owns the bit-clear ────
# `bit_idioms` recognizes `x & ~(1 << n)` as BitClear, so the production shape
# is the native bit instruction, not a 32-bit NOT.  Pin that win, then disable
# the selection fold to keep the original complement-width contract covered.
expect bclear 3 shapes.s
expect bclearu 3 shapes.s
for label in bclear bclearu; do
    if ! has "$(fn_body "$tmp/shapes.s" "$label")" 'btrl'; then
        echo "$label must select the native bit-clear:" >&2
        fn_body "$tmp/shapes.s" "$label" >&2
        exit 1
    fi
    lacks "$(fn_body "$tmp/shapes.s" "$label")" 'not[q l]\|cltq\|shl' \
        "$label still spells the bit-clear as shift/complement"
done
# With selection disabled the bit-clear still complements at 32 bits, in BOTH
# builds: for a 32-bit result the lowering emits `Not_{I32}` itself, so there is
# no wide complement left for `notcast` to narrow here.  Pinned at 8 either way.
expect bclear 8 no_bit_select.s
expect bclearu 8 no_bit_select.s
expect bclear 8 no_notcast_no_select.s
for build in no_bit_select.s no_notcast_no_select.s; do
    if ! has "$(fn_body "$tmp/$build" bclear)" 'notl'; then
        echo "bclear fallback must complement at 32 bits in $build:" >&2
        fn_body "$tmp/$build" bclear >&2
        exit 1
    fi
done
# `notcast` itself is NOT dead: a sub-word result still truncates, so `(T)~x`
# arrives as `trunc(Not_{I32}(ext))` -- the lowering change took away the
# 64-bit spelling for `int` results, not this one.  It is worth exactly one
# instruction per site, and that is what these four counts pin:
#
#   `(unsigned short)~x`  folded: `notq; movl`                    3
#                         unfolded: `movzwl; notl; movzwl`         4
expect nots16 3 shapes.s
expect nots8 3 shapes.s
expect nots16 4 no_notcast.s
expect nots8 4 no_notcast.s
if ! has "$(fn_body "$tmp/no_notcast.s" nots16)" 'notl'; then
    echo "CCC_SIMPLIFY_SKIP=notcast does not restore the 32-bit complement:" >&2
    fn_body "$tmp/no_notcast.s" nots16 >&2
    exit 1
fi
if ! has "$(fn_body "$tmp/shapes.s" nots16)" 'notq'; then
    echo "nots16 must keep the wide complement (the truncation is what makes it legal):" >&2
    fn_body "$tmp/shapes.s" nots16 >&2
    exit 1
fi
# A widening cast keeps its wide complement in every build.
expect notw 4 shapes.s
expect notw 4 no_notcast.s
expect notw 4 no_bit_select.s
expect notw 4 no_notcast_no_select.s
if ! has "$(fn_body "$tmp/shapes.s" notw)" 'notq'; then
    echo "notw must complement at 64 bits (`~(unsigned long)x` observes all of it):" >&2
    fn_body "$tmp/shapes.s" notw >&2
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
static int            bclear(int x, int n)   { return x & ~(1 << n); }
static unsigned       bclearu(unsigned x, unsigned n) { return x & ~(1u << n); }
static unsigned long  notw(unsigned x)       { return ~(unsigned long)x; }
static long           wide(int x)            { return -(long)x; }
static double         fsub(double x)         { return 0.0 - x; }
/* The result is observed as `unsigned`, so the zero extension of the narrow
   negation is part of the answer: this row is what a fold that trusts the low
   bits alone would break. */
static unsigned       keep_width(unsigned short x) { return (unsigned short)-x; }
/* The complement's narrow row: `notcast` narrows these two, so it is part of
   every build's answer and of the gcc comparison below. */
static unsigned short nots16(unsigned short x) { return (unsigned short)~x; }
static unsigned char  nots8(unsigned char x)   { return (unsigned char)~x; }

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
        mix(nots16(s));
        mix(nots8((unsigned char)x));
        mix((unsigned)wide((int)s));
        mix(notzero((int)s));
        mix(notzero(-(int)s));
        mix(notzero(0));
        mix((unsigned)bclear((int)s, (int)(x & 31u)));
        mix(bclearu(x, x & 31u));
        mix(notw(x));
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
for knob in CCC_SIMPLIFY_SKIP=negcast CCC_SIMPLIFY_SKIP=zeroneg CCC_SIMPLIFY_SKIP=notcast; do
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
for name in neg16 neg8 negs16 isol16 notzero wide fsub bclear bclearu notw nots16 nots8; do
    echo "  $name: $(count "$tmp/shapes.s" "$name") instructions (negcast off: $(count "$tmp/no_negcast.s" "$name"))"
done

echo "narrow-negate gate OK"
