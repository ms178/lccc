/* Implicit conversions from and to _Float128 in static initializers, through
 * every initializer path (scalar, array, struct member, compound literal,
 * vector lane, _Bool), plus the same constants folded in function bodies.
 *
 * _Float128 travels in a 16-byte integer carrier, so a conversion chosen by
 * the carrier's IR type instead of the C types reinterprets bits.  Pinned
 * regressions: `static int i = 2.75Q` (was the carrier's low bits), _Float128
 * array/struct/compound-literal elements initialized from integers or
 * doubles (integer -> carrier copy), vector lanes from _Float128,
 * `static _Bool b = -0.0Q` (carrier "nonzero"), double rounding through the
 * double shadow (f128 -> float/long double must round once), and binary128
 * arithmetic in function bodies that called libgcc at run time.
 *
 * All values are exact or correctly rounded; results are printed as bit
 * patterns or exact decimal so nothing depends on printf's rounding. */
#include <stdio.h>
#include <string.h>

typedef _Float128 Q;

static void bits(const char *name, const void *p, size_t n) {
    const unsigned char *b = p;
    printf("%s:", name);
    while (n--)
        printf("%02x", b[n]);
    printf("\n");
}
#define B(v) bits(#v, &(v), sizeof(v))
#if defined(__x86_64__) || defined(__i386__)
#define BL(v) bits(#v, &(v), 10)
#else
#define BL(v) B(v)
#endif

/* --- scalars -------------------------------------------------------- */
static int s_i = 2.75Q;
static int s_ineg = -2.75Q;
static unsigned s_u = 4294967295.75Q;
static long long s_ll = -9223372036854775807.5Q;
static unsigned long long s_ull = 18446744073709551615.25Q;
static unsigned char s_uc = 255.9Q;
static short s_sh = -32768.5Q;
static _Bool s_b1 = 0.5Q, s_b2 = -0.0Q, s_b3 = 0.0Q, s_b4 = 1e-4940Q;
static double s_d = 2.5Q;
static double s_d01 = 0.1Q;
static float s_f01 = 0.1Q;
/* 1 + 2^-24 + 2^-60: once-rounded to float is 1 + 2^-23; rounding through
 * double first gives the tie 1 + 2^-24 and then 1.0. */
static float s_f_dr = 0x1.000001000000001p0Q;
/* Exact ties round to even. */
static float s_f_tie_dn = 0x1.000001p0Q;
static float s_f_tie_up = 0x1.000003p0Q;
/* 1 + 2^-64 + 2^-100: once-rounded to the 64-bit x87 significand is
 * 1 + 2^-63. */
static long double s_ld_dr = 0x1.0000000000000001000000001p0Q;
static long double s_ld01 = 0.1Q;
static Q s_q_ull = 18446744073709551615ULL;
static Q s_q_ll = -9223372036854775807LL - 1;
static Q s_q_d = 0.1;
static Q s_q_f = 0.1f;
static Q s_q_ld = 0.1L;
static Q s_q_sub = 1e-4940Q;       /* subnormal binary128 */
static Q s_q_third = 1.0Q / 3;     /* folded, correctly rounded */
static Q s_q_sum = 2.0Q + 1;       /* integer operand converts to Q */
static Q s_q_neg0 = -0.0Q;
static int s_cmp = (2.5Q > 2) + (1.0Q == 1) * 2 + (0.1Q < 0.1) * 4;
static int s_cond = -0.0Q ? 1 : 2;

/* --- aggregates ----------------------------------------------------- */
static Q s_qa[5] = {1, 2.5, 0.1f, -3LL, 0x1p-16382Q};
static int s_ia[4] = {2.75Q, -3.5Q, 1e3Q, 65535.99Q};
static double s_da[3] = {0.1Q, 1, 1e-320Q};
static _Bool s_ba[4] = {0.0Q, -0.0Q, 1e-4940Q, 2};
static struct {
    int i;
    Q q;
    _Bool b;
    double d;
    float f;
} s_st = {2.75Q, 7, 0.25Q, 0.1Q, 0x1.000001000000001p0Q};
static struct {
    char c;
    Q q[2];
} s_nested[2] = {{1, {2, 3.5}}, {4.0Q, {5U, 0.1}}};
static Q *s_cl = (Q[]){1, 2.5, 0.1};
static int *s_cli = (int[]){2.75Q, -1.5Q};

/* Array typedefs and typeof name the whole array: the element type must
 * come from the object's type, not the declaration's specifier. */
typedef _Bool B3[3];
static B3 s_tb = {2, 0.5, -0.0Q};
typedef Q Q2[2];
static Q2 s_tq = {3, 0.5f};
static __typeof__(s_qa) s_tyq = {7, 0.25};
/* Brace-enclosed scalars. */
static Q s_brq[2] = {{2.75}, {-1}};
static int s_bri[2] = {{2.75Q}, {-1.5Q}};
/* Struct members: _Bool arrays, _Bool from -0.0Q, bit-fields from Q. */
static struct {
    _Bool ba[3];
    _Bool bz;
    int bf : 5;
    unsigned ubf : 3;
    _Bool b1 : 1;
} s_bits = {{2, 0.5, 4}, -0.0Q, -7.75Q, 6.5Q, 0.0Q};
/* A pointer member forces the relocation-bearing struct path. */
static int s_target;
static struct {
    int *p;
    Q q;
    int i;
    _Bool b;
    int bf : 4;
} s_ptrs = {&s_target, 9, 2.75Q, -0.0Q, -3.5Q};
#ifdef __SIZEOF_INT128__
static __int128 s_i128[2] = {2.75Q, -18446744073709551616.5Q};
#endif

typedef float v4f __attribute__((vector_size(16)));
typedef int v4i __attribute__((vector_size(16)));
typedef double v2d __attribute__((vector_size(16)));
static v4f s_v4f = {1.5Q, 2, 0.1Q, 0x1.000001000000001p0Q};
static v4i s_v4i = {2.75Q, -2.75Q, 3, 1e3Q};
static v2d s_v2d = {0.1Q, -0.0Q};

static void statics(void) {
    printf("ints %d %d %u %lld %llu %u %d\n", s_i, s_ineg, s_u, s_ll, s_ull,
           (unsigned)s_uc, (int)s_sh);
    printf("bools %d %d %d %d\n", s_b1, s_b2, s_b3, s_b4);
    printf("cmp %d cond %d\n", s_cmp, s_cond);
    B(s_d);
    B(s_d01);
    B(s_f01);
    B(s_f_dr);
    B(s_f_tie_dn);
    B(s_f_tie_up);
    BL(s_ld_dr);
    BL(s_ld01);
    B(s_q_ull);
    B(s_q_ll);
    B(s_q_d);
    B(s_q_f);
    B(s_q_ld);
    B(s_q_sub);
    B(s_q_third);
    B(s_q_sum);
    B(s_q_neg0);
    for (int i = 0; i < 5; i++)
        bits("s_qa", &s_qa[i], sizeof(Q));
    printf("s_ia %d %d %d %d\n", s_ia[0], s_ia[1], s_ia[2], s_ia[3]);
    for (int i = 0; i < 3; i++)
        bits("s_da", &s_da[i], sizeof(double));
    printf("s_ba %d %d %d %d\n", s_ba[0], s_ba[1], s_ba[2], s_ba[3]);
    printf("s_st.i=%d s_st.b=%d\n", s_st.i, s_st.b);
    B(s_st.q);
    B(s_st.d);
    B(s_st.f);
    for (int i = 0; i < 2; i++) {
        printf("s_nested[%d].c=%d\n", i, s_nested[i].c);
        bits("q0", &s_nested[i].q[0], sizeof(Q));
        bits("q1", &s_nested[i].q[1], sizeof(Q));
    }
    for (int i = 0; i < 3; i++)
        bits("s_cl", &s_cl[i], sizeof(Q));
    printf("s_cli %d %d\n", s_cli[0], s_cli[1]);
    B(s_v4f);
    printf("s_v4i %d %d %d %d\n", s_v4i[0], s_v4i[1], s_v4i[2], s_v4i[3]);
    B(s_v2d);
    printf("s_tb %d %d %d\n", s_tb[0], s_tb[1], s_tb[2]);
    B(s_tq[0]);
    B(s_tq[1]);
    for (int i = 0; i < 5; i++)
        bits("s_tyq", &s_tyq[i], sizeof(Q));
    B(s_brq[0]);
    B(s_brq[1]);
    printf("s_bri %d %d\n", s_bri[0], s_bri[1]);
    printf("s_bits %d %d %d %d %d %d %d\n", s_bits.ba[0], s_bits.ba[1], s_bits.ba[2],
           s_bits.bz, s_bits.bf, s_bits.ubf, s_bits.b1);
    printf("s_ptrs %d %d %d %d\n", s_ptrs.p == &s_target, s_ptrs.i, s_ptrs.b, s_ptrs.bf);
    B(s_ptrs.q);
#ifdef __SIZEOF_INT128__
    printf("s_i128 %lld %lld %llu\n", (long long)s_i128[0], (long long)(s_i128[1] >> 64),
           (unsigned long long)s_i128[1]);
#endif
}

/* Automatic arrays: the local array planner (ArrayGeometry). */
static void autos(void) {
    Q qa[3] = {1, 2.5, 0.1f};
    int ia[3] = {2.75Q, -3.5Q, 1e3Q};
    _Bool ba[3] = {2, -0.0Q, 0.5Q};
    float fa[2] = {0x1.000001000000001p0Q, 0.1Q};
    for (int i = 0; i < 3; i++)
        bits("auto qa", &qa[i], sizeof(Q));
    printf("auto ia %d %d %d ba %d %d %d\n", ia[0], ia[1], ia[2], ba[0], ba[1], ba[2]);
    B(fa);
#ifdef __SIZEOF_INT128__
    __int128 la[2] = {2.75Q, -18446744073709551616.5Q};
    printf("auto la %lld %lld %llu\n", (long long)la[0], (long long)(la[1] >> 64),
           (unsigned long long)la[1]);
#endif
}

/* --- function bodies (folded at compile time unless FENV_ACCESS) -------- */
static void bodies(void) {
    double d = (double)2.5Q;
    float f = (float)0x1.000001000000001p0Q;
    long double ld = (long double)0x1.0000000000000001000000001p0Q;
    Q third = 1.0Q / 3;
    Q sum = 2.0Q + 1;
    Q diff = 0.1Q - 0.1;
    Q prod = 3.0Q * 0.1Q;
    int i = (int)2.75Q;
    unsigned char uc = (unsigned char)255.9Q;
    _Bool b = (_Bool)-0.0Q;
    int truth = (-0.0Q ? 1 : 0) + (!0.0Q) * 2 + (0.0Q || 0) * 4 + (0.5Q && 1) * 8;
    int cmp = (2.5Q > 2) + (1.0Q == 1) * 2 + (0.1Q < 0.1) * 4 + (0.1Q != 0.1f) * 8;
    B(d);
    B(f);
    BL(ld);
    B(third);
    B(sum);
    B(diff);
    B(prod);
    printf("body i=%d uc=%u b=%d truth=%d cmp=%d\n", i, (unsigned)uc, b, truth, cmp);
}

/* --- the same operations at run time (volatile operands) --------------- */
static void runtime(void) {
    volatile Q one = 1, three = 3, q275 = 2.75Q, n0 = -0.0Q;
    volatile Q x = 0x1.000001000000001p0Q, y = 0x1.0000000000000001000000001p0Q;
    Q third = one / three;
    double d = (double)q275;
    float f = (float)x;
    long double ld = (long double)y;
    int i = (int)q275;
    _Bool b = n0;
    B(third);
    B(d);
    B(f);
    BL(ld);
    printf("run i=%d b=%d\n", i, b);
}

int main(void) {
    statics();
    autos();
    bodies();
    runtime();
    return 0;
}
