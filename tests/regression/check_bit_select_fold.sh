#!/usr/bin/env bash
# Single-bit selection: `x | 1 << n`, `x & ~(1 << n)`, `x ^ 1 << n`,
# `(x & 1 << n) != 0`.
#
#   x | (1u << n)          is   bts      set bit n
#   x & ~(1u << n)         is   btr      clear bit n
#   x ^ (1u << n)          is   btc      toggle bit n
#   (x & (1u << n)) != 0   is   bt       test bit n
#
# Portable C spells these four everywhere (bitmaps, atomics masks, allocator
# free lists, `1u << (n & 31)` in hash tables).  Spelled out they cost a
# materialized constant, a variable shift pinned to `%cl` and the logic op;
# both oracles emit one instruction for the set/clear/toggle and two for the
# test, and LCCC emitted 6, 8, 6 and 7 where they emit 3, 3, 3 and 4.
#
# The IR gained `BitSet`, `BitClear` and `BitToggle` (the existing `BitTest`
# covers the test) and `bit_idioms` creates them from those shapes.  The
# toggle came a round later than the other three: `btc` is the one mnemonic of
# the family x86 has that the first round did not lower.  This gate pins six
# things, because an instruction-count win that changes answers is worthless:
#
#   1. every folded shape selects the bit instruction at 32 and 64 bits, with
#      the constant folded away (the shift and the complement are gone),
#   2. the controls keep their exact counts in both builds: a two-bit mask
#      (`3u << n`, under both `|` and `^`), a mask compared against a
#      non-zero constant, and a shift whose result is used twice,
#   3. the equality direction, `(x & 1 << n) == 0`, keeps its count: `BitTest`
#      is a 0/1 value, so the negated form needs a second instruction and the
#      fold would not save the compare,
#   4. `CCC_NO_BIT_SELECT=1` brings every folded shape back to its pinned
#      count and removes every bit mnemonic, which is what proves the
#      assertions above belong to this fold and not to some other rule,
#   5. the other backends never get *longer*: for i686, AArch64 and RISC-V the
#      whole probe corpus is compiled twice (knob off/on) and every function
#      must stay the same length or shrink -- and the native mnemonics appear
#      where the ISA has them (`btsl`/`btrl` on i686, `bic` on AArch64),
#   6. -O0, -O1, -O2, -O3, -Os, the kill switch and GCC agree bit for bit on a
#      checksum over every width, both edges and 64-bit operands.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
bindir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
tmp=${TMPDIR:-/tmp}/lccc-bit-select.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
typedef unsigned u32;
typedef unsigned long u64;

u32 bset32(u32 x, u32 n)     { return x | (1u << n); }
u64 bset64(u64 x, u64 n)     { return x | (1ul << n); }
u32 bset_masked(u32 x, u32 n) { return x | (1u << (n & 31)); }
u32 bclr32(u32 x, u32 n)     { return x & ~(1u << n); }
u64 bclr64(u64 x, u64 n)     { return x & ~(1ul << n); }
int btest32(u32 x, u32 n)    { return (x & (1u << n)) != 0; }
int btest64(u64 x, u64 n)    { return (x & (1ul << n)) != 0; }
u32 btog32(u32 x, u32 n)     { return x ^ (1u << n); }
u64 btog64(u64 x, u64 n)     { return x ^ (1ul << n); }
/* The toggle's involution: `(x ^ m) ^ m` is `x`, and folding both XORs
   without noticing that would turn a 2-instruction identity into two
   `btc`s, which is exactly the pessimisation this row pins. */
u32 btog_twice(u32 x, u32 n) { return x ^ (1u << n) ^ (1u << n); }

/* Shared one-hot mask: on x86 every reader is a bit-family operation, so the
   whole group can drop the materialized mask.  The escaping control keeps the
   mask live and must refuse the group. */
u32 shared_set_tog_test(u32 x, u32 n, u32 *out) {
    u32 m = 1u << n;
    *out = x | m;
    return (x ^ m) + ((x & m) != 0);
}
u32 shared_four(u32 x, u32 n, u32 *out) {
    u32 m = 1u << n;
    out[0] = x | m;
    out[1] = x & ~m;
    out[2] = x ^ m;
    return (x & m) != 0;
}
u32 shared_escape(u32 x, u32 n, u32 *out) {
    u32 m = 1u << n;
    *out = m;
    return (x | m) + (x ^ m);
}
u64 shared64(u64 x, u64 n, u64 *out) {
    u64 m = 1ull << n;
    out[0] = x | m;
    out[1] = x & ~m;
    return (x ^ m) + ((x & m) != 0);
}

/* Controls: none of these is a single-bit operation on the base. */
u32 keep_two(u32 x, u32 n)   { return x & (3u << n); }
u32 keep_xor_two(u32 x, u32 n) { return x ^ (3u << n); }
u32 keep_xor_not(u32 x, u32 n) { return x ^ ~(1u << n); }
u32 keep_shl(u32 x, u32 n)   { return (x << n) | x; }
int keep_eq4(u32 x, u32 n)   { return (x & (1u << n)) == 4; }
int btest32z(u32 x, u32 n)   { return (x & (1u << n)) == 0; }
/* Control: a bit set whose base comes from memory, not a parameter. */
u32 set_from_mem(u32 *p, u32 n) { u32 v = *p; v |= 1u << n; return v; }
C

"$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/on.s"
CCC_NO_BIT_SELECT=1 "$CCC" -O2 -S "$tmp/shapes.c" -o "$tmp/off.s"

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

# ── 1. The folded shapes select the bit instruction, nothing else ──────────
expect bset32 3 on.s
expect bset64 3 on.s
expect bset_masked 4 on.s
expect bclr32 3 on.s
expect bclr64 3 on.s
expect btest32 4 on.s
expect btest64 4 on.s
expect btog32 3 on.s
expect btog64 3 on.s
# The involution stays an identity instead of becoming two bit instructions.
expect btog_twice 2 on.s
expect btog_twice 2 off.s
refuse on.s btog_twice 'btc' "(x ^ 1 << n) ^ (1 << n) must cancel"
want on.s bset32 'bts[lq][[:space:]]'
want on.s bset64 'bts[lq][[:space:]]'
want on.s bclr32 'btr[lq][[:space:]]'
want on.s bclr64 'btr[lq][[:space:]]'
want on.s btest32 'bt[lq][[:space:]]'
want on.s btest64 'bt[lq][[:space:]]'
want on.s btog32 'btc[lq][[:space:]]'
want on.s btog64 'btc[lq][[:space:]]'
# The producers are gone, not merely shadowed by the bit instruction.
refuse on.s bset32 'sh[lq][[:space:]]' "the constant shift must be dead"
refuse on.s bclr32 'sh[lq][[:space:]]' "the constant shift must be dead"
refuse on.s bclr32 'not[lq][[:space:]]' "the complement must be dead"
refuse on.s btest32 'sh[lq][[:space:]]' "the constant shift must be dead"
refuse on.s btest32 'and[lq][[:space:]]' "the mask must be dead"
refuse on.s btog32 'sh[lq][[:space:]]' "the constant shift must be dead"
refuse on.s btog32 'xor[lq][[:space:]]' "the XOR must be the bit instruction"

# Shared-mask groups: x86 selects the whole family and kills the mask producer.
want on.s shared_four 'bts[lq][[:space:]]'
want on.s shared_four 'btr[lq][[:space:]]'
want on.s shared_four 'btc[lq][[:space:]]'
want on.s shared_four 'bt[lq][[:space:]]'
refuse on.s shared_four 'sh[lq][[:space:]]' "the shared mask producer must be dead"
refuse on.s shared_four 'not[lq][[:space:]]' "the shared complement must be dead"
want on.s shared64 'btsq[[:space:]]'
want on.s shared64 'btrq[[:space:]]'
# An escaping mask is not an all-bit group; keep the shared materialization.
refuse on.s shared_escape 'bts[lq][[:space:]]' "escaping mask must not be duplicated"
refuse on.s shared_escape 'btc[lq][[:space:]]' "escaping mask must not be duplicated"

# ── 2. Controls keep their shape in BOTH builds ────────────────────────────
for build in on.s off.s; do
    expect keep_two 6 "$build"
    expect keep_xor_two 6 "$build"
    expect keep_xor_not 8 "$build"
    expect keep_shl 5 "$build"
    expect keep_eq4 8 "$build"
done

# ── 3. The equality direction is not folded ────────────────────────────────
expect btest32z 7 on.s
expect btest32z 7 off.s

# ── 4. The kill switch restores the shift spelling, shape for shape ────────
expect bset32 6 off.s
expect bset64 6 off.s
expect bset_masked 6 off.s
expect bclr32 8 off.s
expect bclr64 8 off.s
expect btest32 7 off.s
expect btest64 7 off.s
expect btog32 6 off.s
expect btog64 6 off.s
for label in bset32 bset64 bclr32 bclr64 btest32 btest64 btog32 btog64 shared_set_tog_test shared_four shared64; do
    refuse off.s "$label" '(^|[[:space:]])bt[s]?[lq]?[[:space:]]' \
        "CCC_NO_BIT_SELECT must remove the bit instruction"
done

# ── 5. The other backends never get longer ────────────────────────────────
# Every function of the corpus, compiled by both builds of each cross target:
# the selected sequence must be the same length or shorter.  That property, not
# a pinned count, is what keeps a backend change from turning this fold into a
# pessimisation.
cross_length_check() { # $1 = binary, $2 = target name
    local bin=$1 name=$2 after before labels label a b
    before="$tmp/$name-off.s"
    after="$tmp/$name-on.s"
    CCC_NO_BIT_SELECT=1 "$bin" -O2 -S "$tmp/shapes.c" -o "$before"
    "$bin" -O2 -S "$tmp/shapes.c" -o "$after"
    labels=$(grep -oE '^[a-z_][a-zA-Z0-9_]*:$' "$before" | tr -d ':')
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
    want i686-on.s bset32 'btsl[[:space:]]'
    want i686-on.s bclr32 'btrl[[:space:]]'
    want i686-on.s btog32 'btcl[[:space:]]'
fi
if [ -x "$bindir/lccc-arm" ]; then
    cross_length_check "$bindir/lccc-arm" arm
    want arm-on.s bclr32 'bic[[:space:]]'
    want arm-on.s btog32 'eor[[:space:]]'
fi
if [ -x "$bindir/lccc-riscv" ]; then
    cross_length_check "$bindir/lccc-riscv" riscv
fi

# ── 6. Answers: every level, both builds, and GCC agree ───────────────────
cat > "$tmp/exec.c" <<'C'
#include <stdio.h>
#include <stdint.h>
typedef unsigned u32;
typedef unsigned long u64;

static u32 bset(u32 x, u32 n)     { return x | (1u << n); }
static u32 bclr(u32 x, u32 n)     { return x & ~(1u << n); }
static int btest(u32 x, u32 n)    { return (x & (1u << n)) != 0; }
static int btestz(u32 x, u32 n)   { return (x & (1u << n)) == 0; }
static u64 bset64(u64 x, u64 n)   { return x | (1ul << n); }
static u64 bclr64(u64 x, u64 n)   { return x & ~(1ul << n); }
static int btest64(u64 x, u64 n)  { return (x & (1ul << n)) != 0; }
static u32 btog(u32 x, u32 n)     { return x ^ (1u << n); }
static u64 btog64(u64 x, u64 n)   { return x ^ (1ul << n); }
static u32 keep_two(u32 x, u32 n) { return x & (3u << n); }
static u32 keep_xor_two(u32 x, u32 n) { return x ^ (3u << n); }
static u32 keep_xor_not(u32 x, u32 n) { return x ^ ~(1u << n); }
static int keep_eq4(u32 x, u32 n) { return (x & (1u << n)) == 4; }
static u32 shared_four(u32 x, u32 n, u32 *o) {
    u32 m = 1u << n;
    o[0] = x | m; o[1] = x & ~m; o[2] = x ^ m;
    return (x & m) != 0;
}
static u32 shared_escape(u32 x, u32 n, u32 *o) {
    u32 m = 1u << n;
    *o = m;
    return (x | m) + (x ^ m);
}
static u64 shared64(u64 x, u64 n, u64 *o) {
    u64 m = 1ull << n;
    o[0] = x | m; o[1] = x & ~m;
    return (x ^ m) + ((x & m) != 0);
}

static unsigned h = 2166136261u;
static void mix(uint64_t v) {
    for (int i = 0; i < 8; i++) h = (h ^ (unsigned)((v >> (8 * i)) & 0xFFu)) * 16777619u;
}

int main(void) {
    static const u32 pat[] = {0u, ~0u, 0xAAAA5555u, 0x80000001u, 0xdeadbeefu};
    for (unsigned p = 0; p < 5; p++) {
        u32 x = pat[p];
        for (u32 n = 0; n < 32; n++) {
            mix(bset(x, n)); mix(bclr(x, n)); mix((unsigned)btest(x, n));
            mix((unsigned)btestz(x, n)); mix(keep_two(x, n)); mix(keep_xor_two(x, n));
            mix(btog(x, n)); mix(btog(btog(x, n), n));
            mix(x ^ (1u << n) ^ (1u << n));
            mix((unsigned)keep_xor_not(x, n));
            mix((unsigned)keep_eq4(x, n));
            { u32 o[3]; mix(shared_four(x, n, o)); mix(o[0]); mix(o[1]); mix(o[2]);
              mix(shared_escape(x, n, o)); mix(o[0]); }
        }
    }
    for (unsigned p = 0; p < 5; p++) {
        u64 x = ((u64)pat[p] << 32) | (u64)pat[p];
        for (u64 n = 0; n < 64; n++) {
            mix(bset64(x, n)); mix(bclr64(x, n)); mix((unsigned)btest64(x, n));
            mix(btog64(x, n)); mix(btog64(btog64(x, n), n));
            { u64 o[2]; mix(shared64(x, n, o)); mix(o[0]); mix(o[1]); }
        }
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
CCC_NO_BIT_SELECT=1 "$CCC" -O2 "$tmp/exec.c" -o "$tmp/exoff"
got=$("$tmp/exoff")
[ "$got" = "$sum" ] || { echo "CCC_NO_BIT_SELECT=1 checksum $got differs from $sum" >&2; exit 1; }
GCC=${GCC:-gcc-16}
if command -v "$GCC" >/dev/null 2>&1; then
    "$GCC" -O2 "$tmp/exec.c" -o "$tmp/exgcc"
    got=$("$tmp/exgcc")
    [ "$got" = "$sum" ] || { echo "$GCC checksum $got differs from $sum" >&2; exit 1; }
fi
CLANG=${CLANG:-clang-23}
if command -v "$CLANG" >/dev/null 2>&1; then
    "$CLANG" -O2 "$tmp/exec.c" -o "$tmp/exclang"
    got=$("$tmp/exclang")
    [ "$got" = "$sum" ] || { echo "$CLANG checksum $got differs from $sum" >&2; exit 1; }
fi
if [ -n "${ICX:-}" ] && command -v "$ICX" >/dev/null 2>&1; then
    "$ICX" -O2 "$tmp/exec.c" -o "$tmp/exicx"
    got=$("$tmp/exicx")
    [ "$got" = "$sum" ] || { echo "$ICX checksum $got differs from $sum" >&2; exit 1; }
fi

echo "OK bit_select_fold ($sum)"
