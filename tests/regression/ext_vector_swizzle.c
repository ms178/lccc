/*
 * Clang ext_vector_type component access ("swizzles"): `v.x`, `v.zyx`,
 * `v.rgba`, `v.s03`, `v.lo/.hi/.even/.odd`, through pointers, on rvalues,
 * chained, as lvalues (single component: any assignment; multi-component:
 * plain assignment), on packed `_Bool` lanes and on 10-byte `long double`
 * lanes.
 *
 * Before: `v.x` on an ext vector resolved like a missing struct field --
 * offset 0, type int -- with no diagnostic, so `q.y = p.x` on a double2
 * compiled to a 4-byte move of the wrong lane (observed: `movl (%rax),%eax`).
 * GCC does not implement ext_vector_type, so this is lccc-only and
 * self-checking (one exit bit per check).
 */
#include <stdio.h>

typedef float F4 __attribute__((ext_vector_type(4)));
typedef float F2 __attribute__((ext_vector_type(2)));
typedef double D2 __attribute__((ext_vector_type(2)));
typedef int I4 __attribute__((ext_vector_type(4)));
typedef long double T3 __attribute__((ext_vector_type(3)));
typedef _Bool B8 __attribute__((ext_vector_type(8)));

__attribute__((noinline)) F4 mk(float a, float b, float c, float d) {
    F4 r = {a, b, c, d};
    return r;
}
__attribute__((noinline)) float sum_hi(const F4 *p) { return p->hi.x + p->hi.y; }
__attribute__((noinline)) void set_odd(F4 *p, F2 v) { p->yw = v; }
/* swizzle-to-swizzle copies read every source lane before the first store:
 * `d == s` and the in-place swap must behave as if through a temporary. */
__attribute__((noinline)) void rev3(F4 *d, const F4 *s) { d->xyz = s->zyx; }
__attribute__((noinline)) void swap_xy(F4 *p) { p->xy = p->yx; }
__attribute__((noinline)) void swap_ld(T3 *p) { p->xz = p->zx; }
/* a vector-returning call as the swizzle source */
__attribute__((noinline)) void from_call(F4 *p) { p->wz = mk(1, 2, 3, 4).xy; }

int main(void) {
    int bad = 0;
    F4 v = {1, 2, 3, 4};
    F2 lo = v.lo, hi = v.hi, ev = v.even, od = v.odd;
    if (lo.x != 1 || lo.y != 2 || hi.x != 3 || hi.y != 4) bad |= 1 << 0;
    if (ev.x != 1 || ev.y != 3 || od.x != 2 || od.y != 4) bad |= 1 << 1;
    F4 rev = v.wzyx, bgra = v.bgra;
    if (rev.x != 4 || rev.y != 3 || rev.z != 2 || rev.w != 1) bad |= 1 << 2;
    if (bgra.x != 3 || bgra.y != 2 || bgra.z != 1 || bgra.w != 4) bad |= 1 << 3;
    F2 s = v.s31;
    if (s.x != 4 || s.y != 2) bad |= 1 << 4;
    if (v.zyx.x != 3 || mk(5, 6, 7, 8).w != 8 || mk(5, 6, 7, 8).yx.x != 6) bad |= 1 << 5;
    F4 *p = &v;
    if (p->y != 2 || sum_hi(&v) != 7) bad |= 1 << 6;
    p->z = 30;
    p->x += 10;
    v.w *= 2;
    v.y++;
    if (v.x != 11 || v.y != 3 || v.z != 30 || v.w != 8) bad |= 1 << 7;
    if (sizeof(v.xy) != sizeof(F2) || sizeof(v.x) != sizeof(float) || sizeof(v.s0123) != sizeof(F4))
        bad |= 1 << 8;

    I4 iv = {1, 2, 3, 4};
    iv.x <<= 4;
    if (iv.wzyx.s0 + iv.hi.y != 8 || iv.x != 16) bad |= 1 << 9;

    D2 dp = {1, 2}, dq = {0, 0};
    dq.y = dp.x;
    dq.x = dp.y;
    if (dq[0] != 2 || dq[1] != 1) bad |= 1 << 10;

    /* multi-component assignment */
    F4 a = {1, 2, 3, 4}, b = {0, 0, 0, 0};
    b.yz = a.zy;
    b.xw = a.lo;
    if (b.x != 1 || b.y != 3 || b.z != 2 || b.w != 2) bad |= 1 << 11;
    set_odd(&b, a.hi);
    if (b.y != 3 || b.w != 4) bad |= 1 << 12;
    F2 r = (b.xy = a.wz);
    if (r.x != 4 || r.y != 3 || b.x != 4 || b.y != 3) bad |= 1 << 13;
    b.zw = 9;
    if (b.z != 9 || b.w != 9) bad |= 1 << 14;

    /* 10-byte x87 lanes: neighbours must survive single-lane stores */
    T3 t = {1.5L, 2.5L, 3.5L};
    t.y = t.z * 2;
    if (t.x != 1.5L || t.y != 7.0L || t.z != 3.5L || t.s1 != 7.0L) bad |= 1 << 15;
    t.x += t.hi.x; /* odd N: hi = lanes {2, 3}; lane 3 is padding (Clang) */
    t.xz = t.zx;
    if (t.x != 3.5L || t.y != 7.0L || t.z != 5.0L) bad |= 1 << 16;

    /* packed bool lanes.  Clang 23.1.4 gives bool ext vectors subscripts
     * only: `m.s1` / `m.lo` are "illegal vector component name" (lccc
     * matches; the diagnostic is pinned by the swizzle diagnostics gate). */
    B8 m = {1, 0, 1, 0, 1, 0, 1, 0};
    m[1] = 1;
    m[0] = 0;
    m[7] = 0;
    if (m[0] + m[1] + m[2] + m[3] + m[4] + m[5] + m[6] + m[7] != 4 || m[0] || m[7] || !m[1]) bad |= 1 << 17;
    if (m[1] != 1 || m[7] != 0 || m[2] != 1 || m[3] != 0 || m[4] != 1 || m[5] != 0 || m[6] != 1)
        bad |= 1 << 18;

    volatile F4 vv = v;
    vv.x = 100;
    if (vv.x != 100 || vv.w != 8) bad |= 1 << 19;

    /* overlapping source and destination */
    F4 o = {1, 2, 3, 4};
    rev3(&o, &o);
    if (o.x != 3 || o.y != 2 || o.z != 1 || o.w != 4) bad |= 1 << 20;
    swap_xy(&o);
    if (o.x != 2 || o.y != 3) bad |= 1 << 21;
    T3 tl = {1.0L, 2.0L, 3.0L};
    swap_ld(&tl);
    if (tl.x != 3.0L || tl.y != 2.0L || tl.z != 1.0L) bad |= 1 << 22;
    from_call(&o);
    if (o.w != 1 || o.z != 2 || o.x != 2) bad |= 1 << 23;
    vv.xy = vv.yx; /* volatile: each lane read once, written once */
    if (vv.x != 3 || vv.y != 100) bad |= 1 << 24;
    /* constant x87 lane stores take the immediate path */
    T3 tc;
    tc.x = 1.0L;
    tc.y = -2.5L;
    tc.z = 0.0L;
    if (tc.x != 1.0L || tc.y != -2.5L || tc.z != 0.0L || tc.hi.x != 0.0L) bad |= 1 << 25;

    printf("swizzle: %s (0x%x)\n", bad ? "FAIL" : "ok", bad);
    return bad != 0; /* an exit status keeps 8 bits: `return bad` hid bits 8+ */
}
