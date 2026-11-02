/* Edge cases of struct/union/array initializers (C11 6.7.9) in block-scope,
 * static and compound-literal storage: unnamed bit-fields skipped by
 * positional initializers, strings overridden element-wise, anonymous
 * unions, _Bool/long double members, arrays of unions, nested struct
 * members with designator continuation, pointer arrays (relocations),
 * unsized compound literals of structs and their sizeof (parsed as
 * `sizeof unary-expression` on the literal), flexible array members with
 * designators, and designated pointer arrays.  A designation into a member
 * initialized by a struct-typed EXPRESSION is left out: gcc discards the
 * expression's other members while clang keeps them, and the standard does
 * not settle it (lccc follows the literal reading of 6.7.9p19, like clang).
 * gcc is the oracle. */
#include <stdio.h>
#include <string.h>
struct B { unsigned a : 3; unsigned : 5; unsigned b : 4; int c; };
struct C { char s[8]; int k; };
struct D { int x; union { int i; char ch[4]; }; int y; };
struct E { double d; _Bool f; long double ld; short h; };
struct F { const char *p[3]; int n; };
union V { struct { int a, b; } s; int arr[3]; };
struct G { union V v[2]; int t; };
struct H { struct C c; struct C c2; };
struct Z { int n; unsigned short s[]; };
struct Zp { const char *p; int w[]; };
static int gx, gy;
static void dump(const char *t, const void *p, size_t n) {
  const unsigned char *b = p; printf("%s", t);
  for (size_t i = 0; i < n; i++) printf(" %02x", b[i]); printf("\n");
}
#define DECLS(ST) \
  ST struct B b1 = { 5, 9, 11 }; \
  ST struct B b2 = { .b = 3, 4 }; \
  ST struct C c1 = { "abc", .s[1] = 'X', 7 }; \
  ST struct C c2 = { .s = "hello", .s[4] = 0, .k = 2, .s = { 'q' } }; \
  ST struct C c3[2] = { "ab", 1, [1].s[2] = 'z', 5 }; \
  ST struct D d1 = { 1, .ch[2] = 7, 9 }; \
  ST struct D d2 = { 1, { .ch = "ab" }, .i = 3 }; \
  ST struct D d3 = { .y = 4, .x = 2, 5 }; \
  ST struct E e1 = { .h = 2, .d = 1.5, 3, 2.25L }; \
  ST struct G g1 = { { [1].arr[2] = 6, [0].s.b = 3 }, 8 }; \
  ST struct G g2 = { .v[1] = { .s = { 1, 2 } }, .v[1].arr[2] = 9 }; \
  ST struct H h1 = { "ab", 1, "cd", 2, .c.s[3] = 'w' }; \
  ST struct H h2 = { .c2 = { "zz" }, .c = c_init, .c2.k = 11 };
static void f(void) {
  struct C c_init = { "init", 42 };
  DECLS()
  printf("b %u %u %d %u %u %d\n", b1.a, b1.b, b1.c, b2.a, b2.b, b2.c); dump("c1", &c1, sizeof c1); dump("c2", &c2, sizeof c2);
  dump("c3", &c3, sizeof c3); dump("d1", &d1, sizeof d1); dump("d2", &d2, sizeof d2); dump("d3", &d3, sizeof d3);
  printf("e1 %g %d %Lg %d\n", e1.d, e1.f, e1.ld, e1.h);
  dump("g1", &g1, sizeof g1); dump("g2", &g2, sizeof g2); dump("h1", &h1, sizeof h1); dump("h2", &h2, sizeof h2);
  struct F f1 = { "a", "b", .p[0] = "c", "d", 0, 3 };
  printf("f1 %s %s %d %d\n", f1.p[0], f1.p[1], f1.p[2] == 0, f1.n);
  struct F f2[2] = { [1].p = { [2] = "z" }, [0].n = 4, [1].p[0] = "y" };
  printf("f2 %d %d %s %s %d\n", f2[0].n, f2[1].p[1] == 0, f2[1].p[0], f2[1].p[2], f2[0].p[0] == 0);
  struct C *cl = (struct C[]){ [2].k = 5, [0] = { "x" }, [1].s = "yy" };
  dump("cl", cl, 3 * sizeof *cl);
  printf("clsz %zu\n", sizeof (struct C[]){ [2].k = 5, [0] = { "x" } });
}
#define c_init { "stat", 43 }
static void g(void) {
  DECLS(static)
  printf("b %u %u %d %u %u %d\n", b1.a, b1.b, b1.c, b2.a, b2.b, b2.c); dump("c1", &c1, sizeof c1); dump("c2", &c2, sizeof c2);
  dump("c3", &c3, sizeof c3); dump("d1", &d1, sizeof d1); dump("d2", &d2, sizeof d2); dump("d3", &d3, sizeof d3);
  printf("e1 %g %d %Lg %d\n", e1.d, e1.f, e1.ld, e1.h);
  dump("g1", &g1, sizeof g1); dump("g2", &g2, sizeof g2); dump("h1", &h1, sizeof h1); dump("h2", &h2, sizeof h2);
  static struct F f1 = { "a", "b", .p[0] = "c", "d", 0, 3 };
  printf("f1 %s %s %d %d\n", f1.p[0], f1.p[1], f1.p[2] == 0, f1.n);
  static struct F f2[2] = { [1].p = { [2] = "z" }, [0].n = 4, [1].p[0] = "y" };
  printf("f2 %d %d %s %s %d\n", f2[0].n, f2[1].p[1] == 0, f2[1].p[0], f2[1].p[2], f2[0].p[0] == 0);
  static struct Z z1 = { 2, { [3] = 7, 1 } };
  static struct Z z2 = { .s = { 1, 2 }, .n = 5 };
  static struct Zp zp = { "q", { [2] = 4 } };
  printf("z1 %d %d %d %d %d %d\n", z1.n, z1.s[0], z1.s[2], z1.s[3], z1.s[4], (int)sizeof z1); printf("z2 %d %d %d\n", z2.n, z2.s[0], z2.s[1]);
  printf("zp %s %d %d %d\n", zp.p, zp.w[0], zp.w[1], zp.w[2]);
  static int *pp[2][2] = { [1] = { &gy }, [0][1] = &gx };
  printf("pp %d %d %d %d\n", pp[0][0] == 0, pp[0][1] == &gx, pp[1][0] == &gy, pp[1][1] == 0);
}
int main(void) { f(); g(); return 0; }
