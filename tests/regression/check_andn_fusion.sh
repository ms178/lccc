#!/usr/bin/env bash
# IS-12: fuse adjacent single-use `not` + `and` only when BMI1 is enabled.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-andn.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
__attribute__((noinline)) unsigned long f(unsigned long a, unsigned long b)
{ return a & ~b; }
int main(void) { return f(0xf0f0UL,0x0ff0UL) != 0xf000UL; }
C
"$CCC" -O2 -march=x86-64-v3 -S "$tmp/t.c" -o "$tmp/bmi.s"
"$CCC" -O2 -march=x86-64-v3 "$tmp/t.c" -o "$tmp/t"
"$tmp/t"
body=$(sed -n '/^f:/,/^\.size f/p' "$tmp/bmi.s")
grep -q 'andnq' <<<"$body"
! grep -q 'notq' <<<"$body"
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/base.s"
! grep -q 'andnq' "$tmp/base.s"  # baseline x86-64 must remain SIGILL-safe

# ── Red-team: the non-negated operand's home read is freshness-gated ──────
# emit_and_not_impl historically read `other`'s home register
# UNCONDITIONALLY: a home clobbered by earlier staging/traffic (spilled
# mask consumed after its home was reused — the free_area_init_node
# class) fed the andn a garbage mask with BMI enabled. The gate below
# builds that shape deterministically: `other` is produced behind a
# noinline call (spilled, slot image), a second call clobbers the
# register region, and only then is `x & ~n` consumed. The result must
# match the scalar reference — a stale-home read yields a wrong product.
cat >"$tmp/t2.c" <<'C'
typedef unsigned long u64;
volatile u64 g_sink;
__attribute__((noinline)) u64 mask_for(int k) {
    return k ? 0x0000ffff0000ffffUL : 0x00ff00ff00ff00ffUL;
}
__attribute__((noinline)) u64 side_effect(u64 v) { g_sink = v; return v; }
__attribute__((noinline)) u64 cmb(u64 x, int pick) {
    u64 m = mask_for(pick);
    u64 side = side_effect(pick + 1);
    u64 n = (pick & 1) ? m : ~m;
    return (x & ~n) ^ (side & 0);
}
__attribute__((noinline)) u64 ref(u64 x, int pick) {
    u64 m = mask_for(pick);
    u64 n = (pick & 1) ? m : ~m;
    return x & ~n;
}
int main(void) {
    for (int p = 0; p < 4; p++) {
        u64 x = 0xf0f0f0f0f0f0f0f0UL ^ (u64)p;
        if (cmb(x, p) != ref(x, p)) return 1;
    }
    return 0;
}
C
"$CCC" -O2 -march=x86-64-v3 "$tmp/t2.c" -o "$tmp/t2"
"$tmp/t2"
"$CCC" -O2 -march=x86-64-v3 -S "$tmp/t2.c" -o "$tmp/t2.s"
grep -q 'andn' "$tmp/t2.s"   # the fusion itself must survive the hardening
