/* Struct, union and array-of-aggregate initializers (C11 6.7.9) that
 * combine brace elision (p20), designator chains, continuation after a
 * designation at the designation's depth (p17), overrides (p19: the last
 * initializer wins, a braced one replaces the whole subobject), anonymous
 * members, union member selection, GNU ranges and unsized arrays of
 * structs, in block-scope, static and compound-literal storage.  lccc
 * resolves them once into a canonical form (src/ir/lowering/init_canon.rs)
 * and arrays of scalars through the array planner (array_init_plan.rs);
 * before, each storage path walked the list on its own and, e.g., put the
 * item after `.in.a = 1` into the member after `in`, placed `[1] = {...}`
 * inside a member array of structs at element 0, and kept `s.b` alive after
 * a later `.a[0]` selected another union member.  gcc is the oracle. */
#include <stdio.h>
struct S { int a[2][3]; int k; };
struct T { struct S s[2]; int z; };
struct I { int a, b; };
struct O { struct I in; int c, d; };
struct A { int g; struct { int x, y; }; int h; };
struct P { const char *p; struct { int k; int m[2]; } in; int q; };
union U { int a[2]; struct I s; long long w; };
static int gv;
static void pr(const char *t, const int *p, int n) { printf("%s", t); for (int i = 0; i < n; i++) printf(" %d", p[i]); printf("\n"); }
#define SZ(x) (int)(sizeof(x)/sizeof(int))
#define DECLS(ST) \
  ST struct S t1[2] = { [0].a[1][2] = 5, 6, [1].k = 7, [1].a[0][1] = 8 }; \
  ST struct T t2 = { .s[1].a[0][2] = 3, 4, .s[0].k = 9, .z = 1 }; \
  ST struct T t3 = { { [1] = { .a = { [1] = {1,2} }, 3 } }, 4 }; \
  ST struct O o1 = { .in.a = 1, 2, 3, 4 }; \
  ST struct O o2[2] = { [1].in.b = 8, 9, [0].c = 1 }; \
  ST struct O o3 = { .in.b = 7, .in = { 1 }, 5 }; \
  ST struct A a1 = { .x = 1, 2, 3 }; \
  ST struct A a2 = { 1, 2, 3, 4 }; \
  ST struct A a3 = { 1, { 5 }, 6 }; \
  ST struct O o4[] = { 1, 2, 3, 4, 5, [3].c = 9 }; \
  ST union U u1 = { .s.b = 4, .a[0] = 3 }; \
  ST union U u2 = { .a[1] = 2, .s = { 7 } }; \
  ST struct O o5[3] = { [0 ... 1] = { { 1, 2 }, 3 }, [1].d = 6 }; \
  ST struct S s6 = { .a[1] = { 4 }, .a[0][2] = 1, 2, 3, 4, 5 }; \
  ST struct S s7 = { 1, 2, 3, 4, 5, 6, 7, .a[1][1] = 0 };
static void f_auto(void) {
  DECLS()
  pr("t1", &t1[0].a[0][0], SZ(t1)); pr("t2", &t2.s[0].a[0][0], SZ(t2)); pr("t3", &t3.s[0].a[0][0], SZ(t3));
  pr("o1", &o1.in.a, SZ(o1)); pr("o2", &o2[0].in.a, SZ(o2)); pr("o3", &o3.in.a, SZ(o3));
  pr("a1", &a1.g, SZ(a1)); pr("a2", &a2.g, SZ(a2)); pr("a3", &a3.g, SZ(a3)); pr("o4", &o4[0].in.a, SZ(o4));
  pr("u1", (int*)&u1, SZ(u1)); pr("u2", (int*)&u2, SZ(u2)); pr("o5", &o5[0].in.a, SZ(o5)); pr("s6", &s6.a[0][0], SZ(s6)); pr("s7", &s7.a[0][0], SZ(s7));
  struct P p1 = { .in.m[1] = 5, .p = "x", 7, 8, 9 };
  printf("p1 %d %d %d %d %d\n", p1.p[0], p1.in.k, p1.in.m[0], p1.in.m[1], p1.q);
  struct P p2 = { .in.k = 1, 2, 3, .p = 0, .q = 4 };
  printf("p2 %d %d %d %d %d\n", p2.p == 0, p2.in.k, p2.in.m[0], p2.in.m[1], p2.q);
}
static void f_static(void) {
  DECLS(static)
  pr("t1", &t1[0].a[0][0], SZ(t1)); pr("t2", &t2.s[0].a[0][0], SZ(t2)); pr("t3", &t3.s[0].a[0][0], SZ(t3));
  pr("o1", &o1.in.a, SZ(o1)); pr("o2", &o2[0].in.a, SZ(o2)); pr("o3", &o3.in.a, SZ(o3));
  pr("a1", &a1.g, SZ(a1)); pr("a2", &a2.g, SZ(a2)); pr("a3", &a3.g, SZ(a3)); pr("o4", &o4[0].in.a, SZ(o4));
  pr("u1", (int*)&u1, SZ(u1)); pr("u2", (int*)&u2, SZ(u2)); pr("o5", &o5[0].in.a, SZ(o5)); pr("s6", &s6.a[0][0], SZ(s6)); pr("s7", &s7.a[0][0], SZ(s7));
  static struct P p1 = { .in.m[1] = 5, .p = "x", 7, 8, 9 };
  printf("p1 %d %d %d %d %d\n", p1.p[0], p1.in.k, p1.in.m[0], p1.in.m[1], p1.q);
  static struct P p2 = { .in.k = 1, 2, 3, .p = (const char *)&gv, .q = 4 };
  printf("p2 %d %d %d %d %d\n", p2.p == (const char*)&gv, p2.in.k, p2.in.m[0], p2.in.m[1], p2.q);
  const int *cl = (const int *)&(struct O){ .in.b = 3, 4, .d = 5 };
  pr("cl", cl, 4);
}
int main(void) { f_auto(); f_static(); return 0; }
