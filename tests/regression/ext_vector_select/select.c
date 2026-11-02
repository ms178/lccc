/* Clang ext-vector element selection (v.x, v.wzyx, p->s3, v.hi): values,
 * assignment, compound assignment, ++/--, nested selection, single
 * evaluation of the base, aliasing operands, and the exact memory footprint
 * of 10-byte x87 long double lanes. (_Bool ext vectors have no selection.)
 * Every printed value is also checked against the result of Clang 23.1.4
 * -O0 (m64 and m32 agree), so the program fails on its own; in addition
 * check_ext_vector_swizzle_diagnostics.sh diffs its output against that Clang
 * at LCCC -O0/-O1/-O2 on -m64 and -m32. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int I1 __attribute__((ext_vector_type(1)));
typedef int I2 __attribute__((ext_vector_type(2)));
typedef int I3 __attribute__((ext_vector_type(3)));
typedef int I4 __attribute__((ext_vector_type(4)));
typedef int I5 __attribute__((ext_vector_type(5)));
typedef int I8 __attribute__((ext_vector_type(8)));
typedef float F4 __attribute__((ext_vector_type(4)));
typedef unsigned char UC4 __attribute__((ext_vector_type(4)));
typedef unsigned U4 __attribute__((ext_vector_type(4)));
typedef long double L2 __attribute__((ext_vector_type(2)));
typedef long double L3 __attribute__((ext_vector_type(3)));

static int failures;
#define CHECK(c)                                                         \
    do {                                                                 \
        if (!(c)) {                                                      \
            printf("FAIL line %d: %s\n", __LINE__, #c);                  \
            failures++;                                                  \
        }                                                                \
    } while (0)

static int calls;
__attribute__((noinline)) static I4 make(int base) {
    calls++;
    I4 v = {base, base + 1, base + 2, base + 3};
    return v;
}

/* Prints the lanes and checks them against Clang's result. */
static void expect_i4(const char *k, I4 v, int x, int y, int z, int w) {
    printf("%s %d %d %d %d\n", k, v.x, v.y, v.z, v.w);
    if (v.x != x || v.y != y || v.z != z || v.w != w) {
        printf("FAIL %s: expected %d %d %d %d\n", k, x, y, z, w);
        failures++;
    }
}

static void selection_values(void) {
    I4 v = {10, 11, 12, 13};
    I8 e = {0, 1, 2, 3, 4, 5, 6, 7};
    expect_i4("wzyx", v.wzyx, 13, 12, 11, 10);
    expect_i4("abgr", v.abgr, 13, 12, 11, 10);
    expect_i4("s3210", v.s3210, 13, 12, 11, 10);
    expect_i4("odd8", e.odd, 1, 3, 5, 7);
    expect_i4("hi8", e.hi, 4, 5, 6, 7);
    I2 h = v.hi, l = v.lo, ev = v.even, od = v.odd;
    printf("halves %d %d %d %d %d %d %d %d\n", h.x, h.y, l.x, l.y, ev.x, ev.y, od.x, od.y);
    CHECK(h.x == 12 && h.y == 13 && l.x == 10 && l.y == 11);
    CHECK(ev.x == 10 && ev.y == 12 && od.x == 11 && od.y == 13);
    I5 five = v.xyzwx;
    printf("five %d %d %d %d %d\n", five.s0, five.s1, five.s2, five.s3, five.s4);
    CHECK(five.s0 == 10 && five.s1 == 11 && five.s2 == 12 && five.s3 == 13 && five.s4 == 10);
    CHECK(v.wzyx.x == 13 && v.xy.y == 11 && v.even.odd == 12);
    CHECK(sizeof(v.xyz) == 16 && sizeof(v.hi) == 8 && sizeof(v.x) == 4 && sizeof(five) == 32);
    I3 t = {20, 21, 22};
    CHECK(t.lo.x == 20 && t.lo.y == 21 && t.hi.x == 22);
    I1 one = {7};
    CHECK(one.x == 7 && one.lo == 7);
    (void)one.hi; /* lane 1 is outside the object: poison, never read */
    F4 f = {0.5f, 1.5f, 2.5f, 3.5f};
    printf("float %g %g\n", (double)(f.wx + f.yz).x, (double)(f.wx * 2.0f).y);
    CHECK((f.wx + f.yz).x == 5.0f && (f.wx * 2.0f).y == 1.0f);
}

static void selection_assignment(void) {
    I4 v = {1, 2, 3, 4};
    v.yx = (I2){20, 10};
    v.hi = (I2){30, 40};
    expect_i4("assign", v, 10, 20, 30, 40);
    v.wz += (I2){1, 2};
    v.xy *= 3;
    expect_i4("compound", v, 30, 60, 32, 41);
    v.yx = v.xy;                       /* operand overlaps the target */
    expect_i4("swap", v, 60, 30, 32, 41);
    v.yx = *(I2 *)&v;                  /* aliasing through memory */
    expect_i4("alias", v, 30, 60, 32, 41);
    v.zw = 5;                          /* scalar broadcast */
    v.x++;
    --v.w;
    expect_i4("incdec", v, 31, 60, 5, 4);
    I2 r = (v.xy = (I2){8, 9});        /* value of a selection assignment */
    printf("value %d %d\n", r.x, r.y);
    CHECK(r.x == 8 && r.y == 9);
    expect_i4("after", v, 8, 9, 5, 4);
    I4 n = {0, 0, 0, 0};
    n.wzyx.zw = (I2){6, 7};            /* nested selection target */
    expect_i4("nested", n, 7, 6, 0, 0);
}

/* Multi-lane compound assignment updates the selected lanes of the root in
 * place, at the element width; its value is the new lanes; padding lanes
 * absorb stores. (Clang has no ++/-- on vectors: see errors.c.) */
static void selection_update(void) {
    I4 v = {1, 2, 3, 4};
    I2 r1 = (v.zx += 1);               /* the new lanes */
    I2 r2 = (v.yw -= 1);
    expect_i4("update", v, 2, 1, 4, 3);
    printf("updvals %d %d %d %d\n", r1.x, r1.y, r2.x, r2.y);
    CHECK(r1.x == 4 && r1.y == 2 && r2.x == 1 && r2.y == 3);
    v.wzyx.xy += 1;                    /* nested target: lanes w and z */
    expect_i4("nupdate", v, 2, 1, 5, 4);
    UC4 c = {250, 255, 0, 1};
    c.xyz += (UC4){10, 1, 255, 0}.xyz; /* lanes wrap at 8 bits */
    c.wy -= 1;
    printf("uchar %d %d %d %d\n", c.x, c.y, c.z, c.w);
    CHECK(c.x == 4 && c.y == 255 && c.z == 255 && c.w == 0);
    U4 u = {100, 7, 0x80000000u, 9};
    u.xz >>= 3;                        /* logical shift */
    u.yw /= (U4){2, 3, 4, 5}.wx;       /* unsigned division: 7/5, 9/2 */
    printf("uint %u %u %u %u\n", u.x, u.y, u.z, u.w);
    CHECK(u.x == 12 && u.y == 1 && u.z == 0x10000000u && u.w == 4);
    I3 t = {20, 21, 22};
    t.odd = (I2){7, 8};                /* lane 3 is padding: absorbs the 8 */
    t.hi += 1;                         /* lanes 2 and 3 */
    printf("i3 %d %d %d\n", t.x, t.y, t.z);
    CHECK(t.x == 20 && t.y == 7 && t.z == 23);
    F4 f = {1.0f, 2.0f, 4.0f, 8.0f};
    f.wy /= 2.0f;
    f.xz -= f.yw;                      /* operand read before the stores */
    printf("fupd %g %g %g %g\n", (double)f.x, (double)f.y, (double)f.z, (double)f.w);
    CHECK(f.x == 0.0f && f.y == 1.0f && f.z == 0.0f && f.w == 4.0f);
    I1 one = {5};
    one.lo += 1;
    printf("one %d\n", one.x);
    CHECK(one.x == 6);
}

static void single_evaluation(void) {
    I4 arr[3] = {{0, 0, 0, 0}, {0, 0, 0, 0}, {0, 0, 0, 0}};
    I4 *p = arr;
    p++->yz = (I2){5, 6};
    p++->x += 9;
    (p++)->w++;
    CHECK(p == arr + 3);
    expect_i4("arr0", arr[0], 0, 5, 6, 0);
    expect_i4("arr1", arr[1], 9, 0, 0, 0);
    expect_i4("arr2", arr[2], 0, 0, 0, 1);
    calls = 0;
    int s = make(100).w + make(200).y;
    I2 m = make(300).zx;
    CHECK(calls == 3 && s == 304 && m.x == 302 && m.y == 300);
    printf("calls %d %d %d %d\n", calls, s, m.x, m.y);
}

static void x87_lanes(void) {
    L3 v;
    memset(&v, 0xEE, sizeof v);
    v.y = 1.0L + 0x1p-60L;
    unsigned char raw[sizeof(L3)];
    memcpy(raw, &v, sizeof v);
    int untouched = 1;
    for (size_t i = 0; i < sizeof raw; i++)
        if ((i < 10 || i >= 20) && raw[i] != 0xEE)
            untouched = 0;
    long double y;
    memcpy(&y, raw + 10, 10);
    CHECK(untouched && y == 1.0L + 0x1p-60L);
    L2 w = {2.5L, -4.25L};
    v.zx = w;
    v.zx += w.yx;
    printf("x87 %d %Lg %Lg %Lg\n", untouched, v.x, v.y, v.z);
    CHECK(v.x == -1.75L && v.y == 1.0L + 0x1p-60L && v.z == -1.75L);
}

int main(void) {
    selection_values();
    selection_assignment();
    selection_update();
    single_evaluation();
    x87_lanes();
    printf("failures %d\n", failures);
    return failures != 0;
}
