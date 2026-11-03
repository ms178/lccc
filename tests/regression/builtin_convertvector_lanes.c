/*
 * __builtin_convertvector over every lane-type pair and vector width the
 * builtin admits: C scalar conversions applied lane by lane.
 *
 * The 4 x i32 <-> 4 x f32 pair has a one-instruction x86-64 form
 * (cvtdq2ps / cvttps2dq).  Every other pair converts lane by lane, and three
 * of them used to be miscompiled through that form, because only the
 * float-or-not bit of each side was checked:
 *  - unsigned int -> float went through the SIGNED cvtdq2ps
 *    (0x80000000u became -2147483648.0f);
 *  - float -> unsigned int went through cvttps2dq (3e9f became 0x80000000);
 *  - 2 x double <-> 2 x long long read the doubles as four floats.
 * Every other shape (8- or 32-byte vectors, integer <-> integer,
 * float <-> double, _Float16 lanes) stopped the compiler with an internal
 * error.  A _Float16 lane converts by value, not as its 16-bit carrier.
 *
 * Inputs stay inside each conversion's defined range (no float -> int
 * overflow, no negative float -> unsigned, no float narrowing beyond
 * FLT_MAX), so GCC 16.2 is a valid oracle for every printed line.
 */
#include <stdio.h>

typedef int v4si __attribute__((vector_size(16)));
typedef unsigned v4su __attribute__((vector_size(16)));
typedef float v4sf __attribute__((vector_size(16)));
typedef double v2df __attribute__((vector_size(16)));
typedef long long v2di __attribute__((vector_size(16)));
typedef unsigned long long v2du __attribute__((vector_size(16)));
typedef short v8hi __attribute__((vector_size(16)));
typedef unsigned short v8hu __attribute__((vector_size(16)));
typedef signed char v16qi __attribute__((vector_size(16)));
typedef unsigned char v16qu __attribute__((vector_size(16)));
typedef short v4hi __attribute__((vector_size(8)));
typedef int v2si __attribute__((vector_size(8)));
typedef float v2sf __attribute__((vector_size(8)));
typedef signed char v8qi __attribute__((vector_size(8)));
typedef unsigned char v8qu __attribute__((vector_size(8)));
typedef double v4df __attribute__((vector_size(32)));
typedef int v8si __attribute__((vector_size(32)));
typedef long long v4di __attribute__((vector_size(32)));
typedef float v8sf __attribute__((vector_size(32)));
typedef _Float16 v8hf __attribute__((vector_size(16)));

/* Opaque inputs: the conversions must happen at run time. */
static volatile int vi[4] = {-7, 16777217, -2147483647 - 1, 2147483647};
static volatile unsigned vu[4] = {0x80000000u, 0xffffffffu, 1u, 0xffffff7fu};
static volatile float vf[4] = {-1.9f, 2147483520.0f, 100.5f, -0.0f};
static volatile float vfu[4] = {3e9f, 4294967040.0f, 0.75f, 16777216.0f};
static volatile double vd[4] = {-3.99, 1e15, 1e-50, 3.4028234663852886e38};
static volatile long long vl[2] = {(1LL << 53) + 1, -5};
static volatile unsigned long long vul[2] = {0xffffffffffffffffull, 1ull << 63};
static volatile short vs[8] = {-1, -32768, 32767, 0, 1, -2, 300, -300};
static volatile signed char vc[16] = {-128, -1, 0, 1, 127, 64, -64, 3,
                                      -3, 100, -100, 9, -9, 50, -50, 7};

__attribute__((noinline)) static v4sf i32_to_f32(v4si a) { return __builtin_convertvector(a, v4sf); }
__attribute__((noinline)) static v4si f32_to_i32(v4sf a) { return __builtin_convertvector(a, v4si); }
__attribute__((noinline)) static v4sf u32_to_f32(v4su a) { return __builtin_convertvector(a, v4sf); }
__attribute__((noinline)) static v4su f32_to_u32(v4sf a) { return __builtin_convertvector(a, v4su); }
__attribute__((noinline)) static v2di f64_to_i64(v2df a) { return __builtin_convertvector(a, v2di); }
__attribute__((noinline)) static v2df i64_to_f64(v2di a) { return __builtin_convertvector(a, v2df); }
__attribute__((noinline)) static v2df u64_to_f64(v2du a) { return __builtin_convertvector(a, v2df); }
__attribute__((noinline)) static v8hu s16_to_u16(v8hi a) { return __builtin_convertvector(a, v8hu); }
__attribute__((noinline)) static v16qu s8_to_u8(v16qi a) { return __builtin_convertvector(a, v16qu); }
__attribute__((noinline)) static v2sf f64_to_f32_narrow(v2df a) { return __builtin_convertvector(a, v2sf); }
__attribute__((noinline)) static v2si make_v2si(int a, int b) { return (v2si){a, b}; }

int main(void)
{
	v4si i = {vi[0], vi[1], vi[2], vi[3]};
	v4su u = {vu[0], vu[1], vu[2], vu[3]};
	v4sf f = {vf[0], vf[1], vf[2], vf[3]};
	v4sf fu = {vfu[0], vfu[1], vfu[2], vfu[3]};
	v4df d = {vd[0], vd[1], vd[2], vd[3]};
	v2df d2 = {vd[0], vd[1]};
	v2di l = {vl[0], vl[1]};
	v2du ul = {vul[0], vul[1]};
	v8hi s = {vs[0], vs[1], vs[2], vs[3], vs[4], vs[5], vs[6], vs[7]};
	v16qi c;
	for (int k = 0; k < 16; k++)
		c[k] = vc[k];

	/* Across a call boundary: the operand and the result are by value. */
	v4sf a = i32_to_f32(i);
	printf("i32->f32 %.1f %.1f %.1f %.1f\n", a[0], a[1], a[2], a[3]);
	v4si b = f32_to_i32(f);
	printf("f32->i32 %d %d %d %d\n", b[0], b[1], b[2], b[3]);
	v4sf e = u32_to_f32(u);
	printf("u32->f32 %.1f %.1f %.1f %.1f\n", e[0], e[1], e[2], e[3]);
	v4su g = f32_to_u32(fu);
	printf("f32->u32 %u %u %u %u\n", g[0], g[1], g[2], g[3]);
	v2di h = f64_to_i64(d2);
	printf("f64->i64 %lld %lld\n", h[0], h[1]);
	v2df m = i64_to_f64(l);
	printf("i64->f64 %.1f %.1f\n", m[0], m[1]);
	v2df n = u64_to_f64(ul);
	printf("u64->f64 %.1f %.1f\n", n[0], n[1]);
	v8hu o = s16_to_u16(s);
	printf("s16->u16 %u %u %u %u %u %u %u %u\n", o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7]);
	v16qu p = s8_to_u8(c);
	printf("s8->u8");
	for (int k = 0; k < 16; k++)
		printf(" %u", p[k]);
	printf("\n");
	v2sf q = f64_to_f32_narrow(d2);
	printf("f64->f32 %a %a\n", q[0], q[1]);

	/* Inline, at every width: 16 -> 8, 8 -> 16, 8 -> 32, 16 -> 32, 32 -> 16. */
	v4hi r = __builtin_convertvector(i, v4hi);
	printf("i32->i16 %d %d %d %d\n", r[0], r[1], r[2], r[3]);
	v4si t = __builtin_convertvector(r, v4si);
	printf("i16->i32 %d %d %d %d\n", t[0], t[1], t[2], t[3]);
	v8qu w8 = {200, 1, 255, 0, 128, 127, 9, 90};
	v8si x = __builtin_convertvector(w8, v8si);
	printf("u8->i32 %d %d %d %d %d %d %d %d\n", x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);
	v8qi y8 = {-128, -1, 0, 1, 127, 64, -64, 3};
	v8sf y = __builtin_convertvector(y8, v8sf);
	printf("s8->f32 %.1f %.1f %.1f %.1f %.1f %.1f %.1f %.1f\n", y[0], y[1], y[2], y[3], y[4], y[5], y[6], y[7]);
	v4df z = __builtin_convertvector(f, v4df);
	printf("f32->f64 %a %a %a %a\n", z[0], z[1], z[2], z[3]);
	v4sf zz = __builtin_convertvector(d, v4sf);
	printf("f64->f32 %a %a %a %a\n", zz[0], zz[1], zz[2], zz[3]);
	v4df id = __builtin_convertvector(i, v4df);
	printf("i32->f64 %.1f %.1f %.1f %.1f\n", id[0], id[1], id[2], id[3]);
	v4df dd = {-3.99, 123456789.75, -2147483648.0, 0.5};
	v4si di = __builtin_convertvector(dd, v4si);
	printf("f64->i32 %d %d %d %d\n", di[0], di[1], di[2], di[3]);
	v4di wl = __builtin_convertvector(u, v4di);
	printf("u32->i64 %lld %lld %lld %lld\n", wl[0], wl[1], wl[2], wl[3]);
	v4si same = __builtin_convertvector(i, v4si);
	v4su re = __builtin_convertvector(i, v4su);
	printf("same %d %d | reinterp %u %u\n", same[0], same[3], re[0], re[2]);

	/* _Float16 lanes: 2049 rounds to 2048 (11-bit significand, ties to even). */
	v8hi hs = {-3, 0, 1, 2048, -2049, 65, 7, -7};
	v8hf hf = __builtin_convertvector(hs, v8hf);
	v8sf hw = __builtin_convertvector(hf, v8sf);
	printf("i16->f16->f32 %.1f %.1f %.1f %.1f %.1f %.1f %.1f %.1f\n", hw[0], hw[1], hw[2], hw[3], hw[4], hw[5], hw[6], hw[7]);
	v8hi hb = __builtin_convertvector(hf, v8hi);
	printf("f16->i16 %d %d %d %d %d %d %d %d\n", hb[0], hb[1], hb[2], hb[3], hb[4], hb[5], hb[6], hb[7]);

	/* A small vector returned in a register is spilled to an address. */
	v2sf k2 = __builtin_convertvector(make_v2si(-9, 41), v2sf);
	printf("v2si->v2sf %.1f %.1f\n", k2[0], k2[1]);
	v2si k3 = __builtin_convertvector(k2, v2si);
	printf("v2sf->v2si %d %d\n", k3[0], k3[1]);
	return 0;
}
