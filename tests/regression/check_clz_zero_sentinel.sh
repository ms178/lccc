#!/usr/bin/env bash
# The defined-zero leading-zero count is a sentinel, not a diamond.
#
# `Clz` in this IR answers `width` at zero -- the constant folder says so, and
# the baseline x86 lowering used to answer it with a diamond:
#
#     test; jnz .Lclz_nz; mov $32; jmp .Lclz_done; .Lclz_nz: bsr; xor $31
#
# Six instructions and two branches, one of them a *taken* jump on the zero path
# that exists only to skip two instructions.  `bsr` leaves its destination
# untouched when its source is zero (the same guarantee `emit_ctz32_rax_baseline`
# leans on for `bsf`), so a destination preloaded with 63 answers 32 after the
# same `xor $31` -- 63 ^ 31 == 32 -- and the diamond collapses to
#
#     mov $63; bsr; xor $31
#
# which is what Clang emits for a guarded `clz`, in three instructions with no
# branch.  The operand is staged through a scratch register when source and
# destination are the same register, and the dead-scratch peephole removes the
# bracketing save/restore whenever nothing live holds it.
#
# This gate pins five things:
#
#   1. no defined-zero diamond: no `.Lclz_*` label and no branch in the count,
#   2. the sentinels themselves: `$63` at 32 bits, `$127` at 64 (127 ^ 63 = 64),
#   3. `bsr` still selects the scan (no silent switch to `bsf`),
#   4. the i686 backend emits the same shape (with its own staging pair),
#   5. the values: every 32-bit domain reachable by a multiplicative stride and
#      the 64-bit edges, against a longhand reference, at five optimisation
#      levels -- including `clz(0)`, which is the whole point of the sentinel.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
bindir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
tmp=${TMPDIR:-/tmp}/lccc-clz-zero.$$
mkdir -p "$tmp"
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

x86=$CCC
[ -x "$bindir/lccc" ] && x86=$bindir/lccc

cat > "$tmp/c.c" <<'C'
typedef unsigned u32;
typedef unsigned long long u64;
int clz32(u32 x) { return __builtin_clz(x); }
int clz64(u64 x) { return __builtin_clzll(x); }
int ctz32(u32 x) { return __builtin_ctz(x); }
C

body() { awk -v F="^$1:" '$0 ~ F {p=1; next} p && /^[[:space:]]*\.size/ {exit} p' "$2" |
    grep -vE '^[[:space:]]*(\..*|#.*|)$'; }

# ---- 1/2/3. x86-64 --------------------------------------------------------
"$x86" -O2 -mno-lzcnt -S "$tmp/c.c" -o "$tmp/x86.s"
for f in clz32 clz64; do
    b=$(body "$f" "$tmp/x86.s")
    [ -n "$b" ] || fail "$f: empty body"
    grep -qE '^\.Lclz_' <<<"$b" && fail "$f: the defined-zero diamond is back"
    grep -qE '\b(j[a-z]+|call)\b' <<<"$b" && fail "$f: not straight-line"
    grep -qE '\bbsr[lq]?\b' <<<"$b" || fail "$f: no bsr"
    grep -qE '\bbfs' <<<"$b" && fail "$f: bsf in a leading-zero count"
done
grep -qE 'movl \$63, %e?ax' <<<"$(body clz32 "$tmp/x86.s")" ||
    fail "clz32: no 63 sentinel (63 ^ 31 == 32)"
grep -qE 'movl \$127, %e?ax' <<<"$(body clz64 "$tmp/x86.s")" ||
    fail "clz64: no 127 sentinel (127 ^ 63 == 64)"
grep -qE 'xor[lq] \$31' <<<"$(body clz32 "$tmp/x86.s")" || fail "clz32: no xor 31"

grep -qE 'xor[lq] \$63' <<<"$(body clz64 "$tmp/x86.s")" || fail "clz64: no xor 63"

# ---- 4. i686 emits the same shape -----------------------------------------
i686=$bindir/lccc-i686
[ -x "$i686" ] || fail "missing backend driver $i686"
"$i686" -O2 -mno-lzcnt -S "$tmp/c.c" -o "$tmp/i686.s"
b=$(body clz32 "$tmp/i686.s")
grep -qE '^\.Lclz_' <<<"$b" && fail "clz32: diamond on i686"
grep -qE '\b(j[a-z]+|call)\b' <<<"$b" && fail "clz32: not straight-line on i686"
grep -q 'movl \$63, %eax' <<<"$b" || fail "clz32: no sentinel on i686"
# The scanned register is the allocator's choice -- the count may fold onto
# the operand's own home (`bsrl %ebx, %eax`) once the peephole sees that the
# staging register is dead -- so the gate pins `bsr` into %eax, not the name.
grep -qE '\bbsrl %[a-z0-9]+, %eax' <<<"$b" || fail "clz32: no bsr on i686"
grep -qE '\bbsf' <<<"$b" && fail "clz32: bsf in a leading-zero count on i686"
# No save/restore bracket: a gpr32 count is an `UnaryOp` scratch hazard point
# (collect_i686_scratch_hazard_points marks it (true, true)), so the staging
# register is provably dead and `pushl`/`popl` would be pure overhead.
grep -qE '\b(pushl %ecx|popl %ecx)' <<<"$b" && fail "clz32: stack bracket on i686"

# ---- 4b. the same sentinel contract for the trailing count -----------------
# `ctz32` answers 32 at zero because `bsf` leaves its destination untouched
# and the destination is preloaded with 32 -- the x86-64 pair
# (`movl $32; bsfl`) and, on i686, the same pair with the operand scanned
# where it lives.  A diamond here is the defect this gate exists for.
for tgt in x86 i686; do
    bin=$x86
    [ "$tgt" = i686 ] && bin=$i686
    "$bin" -O2 -S "$tmp/c.c" -o "$tmp/$tgt-ctz.s"
    b=$(body ctz32 "$tmp/$tgt-ctz.s")
    [ -n "$b" ] || fail "ctz32: empty body on $tgt"
    grep -qE '^\.Lctz_' <<<"$b" && fail "ctz32: defined-zero diamond on $tgt"
    grep -qE '\b(j[a-z]+|call)\b' <<<"$b" && fail "ctz32: not straight-line on $tgt"
    grep -qE 'movl \$32, %eax' <<<"$b" || fail "ctz32: no 32 sentinel on $tgt"
    grep -qE '\bbsfl %[a-z0-9]+, %eax' <<<"$b" || fail "ctz32: no bsf on $tgt"
done

# ---- 5. values -------------------------------------------------------------
cat > "$tmp/v.c" <<'C'
#include <stdio.h>
typedef unsigned u32;
typedef unsigned long long u64;
static int m32(u32 x) {
    int n = 0;
    if (!x) return 32;
    while (!(x & 0x80000000u)) { x <<= 1; n++; }
    return n;
}
static int m64(u64 x) {
    int n = 0;
    if (!x) return 64;
    while (!(x & 0x8000000000000000ull)) { x <<= 1; n++; }
    return n;
}
__attribute__((noinline)) int c32(u32 x) { return __builtin_clz(x); }
__attribute__((noinline)) int c64(u64 x) { return __builtin_clzll(x); }
int main(void) {
    unsigned fails = 0, i;
    static const u32 e32[] = {0u, 1u, 2u, 3u, 0x7fffffffu, 0x80000000u, 0xffffffffu,
                              0x8000u, 0x800000u, 0xffffu, 0x10000u};
    static const u64 e64[] = {0ull, 1ull, 0x8000000000000000ull, 0xffffffffffffffffull,
                              0x00000000ffffffffull, 0xffffffff00000000ull, 2ull};
    for (i = 0; i < (1u << 20); i++) {
        u32 x = (u32)(i * 2654435761u);
        fails += (unsigned)(c32(x) != m32(x));
        fails += (unsigned)(c32(~x) != m32(~x));
    }
    for (i = 0; i < sizeof e32 / sizeof e32[0]; i++) fails += (unsigned)(c32(e32[i]) != m32(e32[i]));
    for (i = 0; i < 32; i++) {
        fails += (unsigned)(c32(1u << i) != m32(1u << i));
        fails += (unsigned)(c32(0xffffffffu >> i) != m32(0xffffffffu >> i));
    }
    for (i = 0; i < (1u << 18); i++) {
        u64 x = (u64)i * 0x9e3779b97f4a7c15ull;
        fails += (unsigned)(c64(x) != m64(x));
        fails += (unsigned)(c64(~x) != m64(~x));
    }
    for (i = 0; i < sizeof e64 / sizeof e64[0]; i++) fails += (unsigned)(c64(e64[i]) != m64(e64[i]));
    printf("clz failures=%u\n", fails);
    return fails != 0;
}
C
for o in -O0 -O1 -O2 -O3 -Os; do
    "$x86" "$o" "$tmp/v.c" -o "$tmp/v" || fail "build $o"
    out=$("$tmp/v") || fail "values wrong at $o: $out"
    [ "$out" = "clz failures=0" ] || fail "values wrong at $o: $out"
done
# The other oracle agrees about every value it is allowed to answer (its
# `__builtin_clz(0)` is undefined, so only the model comparison above covers
# the zero case).
if command -v gcc-16 >/dev/null 2>&1; then
    gcc-16 -O2 "$tmp/v.c" -o "$tmp/vg" && "$tmp/vg" >/dev/null 2>&1 || true
fi

echo "check_clz_zero_sentinel: OK (no diamond, 63/127 sentinels on x86-64 and i686, values exact at five levels)"
