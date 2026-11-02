/* Static and automatic initializers of vectors inside aggregates, byte for
 * byte against GCC 16.2 (tests/regression/check_vector_static_init.sh).
 * Covers the byte-image and relocation (pointer-bearing) static paths, brace
 * elision into vector members, arrays of vectors, unions, nested records,
 * compound literals, same-type casts and integer-to-vector casts. */
#include <stdio.h>
typedef double T __attribute__((vector_size(16)));
typedef float F __attribute__((vector_size(16)));
typedef int I __attribute__((vector_size(16)));
typedef char C __attribute__((vector_size(4)));
typedef float F2 __attribute__((vector_size(8)));
typedef double D1 __attribute__((vector_size(8)));
typedef struct { T v; } S;
typedef struct { int i; F v; } S2;
typedef struct { D1 v; } S3;
typedef struct { C c; F2 f; short k; } S4;
typedef struct { S s; int k; } N;
typedef struct { void *p; T v; int k; } P;
typedef union { I v; long long l; } U;
static int anchor;
S a1 = { { 7.0, 10.0 } };
S a2 = { (T){ 7.5, 11.0 } };
S a3 = { 3.0, 4.0 };
S a4 = { .v = { 1.25, 2.5 } };
S a5 = { { 9.0 } };
S2 b1 = { 1, { 1.f, 2.f, 3.f, 4.f } };
S2 b2 = { .v = { 5.f, 6.f }, .i = 9 };
S2 b3 = { 2, 3.f, 4.f };
S3 c1 = { { 5.0 } };
S4 d1 = { { 1, 2, 3, 4 }, { 1.5f, 2.5f }, 77 };
S4 d2 = { 9, 8, 7, 6, 0.5f, 0.25f, 66 };
N e1 = { { { 1.0, 2.0 } }, 3 };
N e2 = { 4.0, 5.0, 6 };
P f1 = { &anchor, { 1.0, 2.0 }, 5 };
P f2 = { 0, (T){ 3.0, 4.0 }, 6 };
P f3 = { &anchor, 7.0, 8.0, 9 };
U g1 = { { 1, 2, 3, 4 } };
U g2 = { .l = 5 };
S h1[2] = { { { 1.0, 2.0 } }, { { 3.0, 4.0 } } };
S h2[2] = { 1.0, 2.0, 3.0, 4.0 };
S2 h3[2] = { [1] = { 7, { 1.f } }, [0].v = { 2.f, 3.f } };
T i1 = (T){ 1.0, 2.0 };
F i2[2] = { (F){ 1.f, 2.f, 3.f, 4.f }, { 5.f } };
const S j1 = { { -1.0, -2.0 } };
C m6 = (C)0x01020304;
typedef struct { F a[2]; int k; } AV;
AV m7 = { { { 1.f, 2.f }, { 3.f } }, 4 };
AV m8 = { 1.f, 2.f, 3.f, 4.f, 5.f, 6.f, 7.f, 8.f, 9 };
typedef struct { void *p; F a[2]; } PAV;
PAV m9 = { &anchor, { { 1.f }, (F){ 2.f, 3.f } } };
T n1 = (T)(T){ 1.5, 2.5 };
T n2 = (T)(__int128)5;
S n3 = { (T)(__int128)0x0102030405060708LL };
C n4 = (C)(int)7;
typedef union { F v; double d; } UV;
UV n5 = { { 1.f, 2.f, 3.f, 4.f } };
/* Pointer declarators whose base specifier is a vector type: the object's
 * type, not the specifier, decides how the initializer is lowered. */
T mkv(int k) { T r = { k, k }; return r; }
T *q1 = { &i1 };
T *q2[2] = { &i1, 0 };
T (*q3)(int) = mkv;
T (*q4[1])(int) = { mkv };
static void dump(const char *n, const void *p, unsigned long sz) {
  const unsigned char *b = p;
  printf("%s:", n);
  for (unsigned long i = 0; i < sz; i++) printf("%02x", b[i]);
  printf("\n");
}
#define D(x) dump(#x, &x, sizeof x)
#define DPA(x) do { PAV t_ = x; t_.p = (void *)(t_.p == &anchor); dump(#x, &t_, sizeof t_); } while (0)
#define DP(x) do { P t_ = x; t_.p = (void *)(t_.p == &anchor); dump(#x, &t_, sizeof t_); } while (0)
int main(void) {
  static S k1 = { { 11.0, 12.0 } };
  static S2 k2 = { 3, { 0.5f, 0.75f } };
  static P k3 = { &anchor, { 13.0, 14.0 }, 15 };
  D(a1); D(a2); D(a3); D(a4); D(a5); D(b1); D(b2); D(b3); D(c1); D(d1); D(d2);
  D(e1); D(e2); DP(f1); DP(f2); DP(f3); D(g1); D(g2); D(h1); D(h2); D(h3);
  D(i1); D(i2); D(j1); D(k1); D(k2); DP(k3);
  D(m6); D(n1); D(n2); D(n3); D(n4); D(n5); D(m7); D(m8); DPA(m9);
  {
    S la3 = { 3.0, 4.0 }; S2 lb2 = { .v = { 5.f, 6.f }, .i = 9 }; S4 ld2 = { 9, 8, 7, 6, 0.5f, 0.25f, 66 };
    N le2 = { 4.0, 5.0, 6 };
    AV lm8 = { 1.f, 2.f, 3.f, 4.f, 5.f, 6.f, 7.f, 8.f, 9 };
    /* Automatic objects: members only -- their padding is indeterminate. */
    D(la3); D(lb2.i); D(lb2.v); D(ld2.c); D(ld2.f); D(ld2.k); D(le2.s); D(le2.k);
    D(lm8.a); D(lm8.k);
  }
  printf("ptrs:%d%d%d%d%d\n", q1 == &i1, q2[0] == &i1, q2[1] == 0, q3 == mkv, q4[0] == mkv);
  return 0;
}
