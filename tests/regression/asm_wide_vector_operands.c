/* `asm` operands that are 256-bit vectors, in the register classes "x" and
 * "v".
 *
 * lccc typed such an operand as a pointer, moved eight bytes of it into an
 * `xmm` register and printed the `xmm` name: `vpaddd %xmm2, %xmm1, %xmm0`
 * for three `__m256i`, wrong values at every optimisation level and no
 * diagnostic.  Each statement below is one way such an operand is written;
 * the expected values are computed in C.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

#define NOINLINE __attribute__((noinline))

static int failures;

static void expect(const char *what, const void *got, const void *want, size_t bytes)
{
    if (memcmp(got, want, bytes) != 0) {
        printf("FAIL %s\n", what);
        failures++;
    }
}

static __m256i load(const int *p) { return _mm256_loadu_si256((const __m256i *)p); }
static void store(int *p, __m256i v) { _mm256_storeu_si256((__m256i *)p, v); }

/* Three registers. */
NOINLINE static void add(int *out, const int *a, const int *b)
{
    __m256i va = load(a), vb = load(b), r;
    __asm__("vpaddd %2, %1, %0" : "=x"(r) : "x"(va), "v"(vb));
    store(out, r);
}

/* An input tied to the output: one register for both. */
NOINLINE static void tied(int *out, const int *a, const int *b)
{
    __m256i va = load(a), vb = load(b), r;
    __asm__("vpsubd %2, %0, %0" : "=x"(r) : "0"(va), "x"(vb));
    store(out, r);
}

/* A read-write operand. */
NOINLINE static void read_write(int *out, const int *a, const int *b)
{
    __m256i acc = load(a), vb = load(b);
    __asm__("vpmulld %1, %0, %0" : "+x"(acc) : "x"(vb));
    store(out, acc);
}

/* Operands by name, and the `xmm` view of a `ymm` operand. */
NOINLINE static void halves(int *out, const int *a)
{
    __m256i va = load(a), r;
    __asm__("vextracti128 $1, %[in], %x[out]\n\t"
            "vpaddd %x[in], %x[out], %x[out]"
            : [out] "=&x"(r) : [in] "x"(va));
    store(out, r);
}

/* An operand that is no lvalue. */
NOINLINE static void rvalue(int *out, const int *a, const int *b)
{
    __m256i va = load(a), vb = load(b), r;
    __asm__("vpslld $1, %1, %0" : "=x"(r) : "x"(_mm256_add_epi32(va, vb)));
    store(out, r);
}

/* Memory beside the register: either is right, the value is the same. */
NOINLINE static void register_or_memory(int *out, const int *a, const int *b)
{
    __m256i va = load(a), vb = load(b), r;
    __asm__("vpaddd %2, %1, %%ymm0\n\tvmovdqu %%ymm0, %0"
            : "=xm"(r) : "x"(va), "xm"(vb) : "xmm0");
    store(out, r);
}

/* A register the template names is not handed out.  lccc hands them out
 * from the last one down, so the last two of the mode are the ones to name. */
#ifdef __x86_64__
#define LAST "15"
#define LAST_BUT_ONE "14"
#else
#define LAST "7"
#define LAST_BUT_ONE "6"
#endif

NOINLINE static void named_register(int *out, const int *a, const int *b)
{
    __m256i va = load(a), vb = load(b), r;
    __asm__("vmovdqa %1, %%ymm" LAST "\n\t"
            "vpxor %%ymm" LAST_BUT_ONE ", %%ymm" LAST_BUT_ONE ", %%ymm" LAST_BUT_ONE "\n\t"
            "vpsubd %2, %%ymm" LAST_BUT_ONE ", %%ymm" LAST_BUT_ONE "\n\t"
            "vpaddd %%ymm" LAST ", %%ymm" LAST_BUT_ONE ", %0"
            : "=x"(r) : "x"(va), "x"(vb) : "xmm" LAST, "xmm" LAST_BUT_ONE);
    store(out, r);
}

/* Members, pointees and array elements. */
struct box {
    int tag;
    __m256i v;
};

NOINLINE static void places(struct box *dst, const struct box *src, __m256i *p, __m256i arr[2])
{
    __asm__("vpaddd %2, %1, %0\n\tvpaddd %3, %0, %0"
            : "=&x"(dst->v) : "x"(src->v), "x"(*p), "x"(arr[1]));
}

/* Floating-point vectors. */
NOINLINE static void doubles(double *out, const double *a, const double *b)
{
    __m256d va = _mm256_loadu_pd(a), vb = _mm256_loadu_pd(b), r;
    __asm__("vmulpd %2, %1, %0" : "=x"(r) : "x"(va), "x"(vb));
    _mm256_storeu_pd(out, r);
}

NOINLINE static void floats(float *out, const float *a, const float *b)
{
    __m256 va = _mm256_loadu_ps(a), vb = _mm256_loadu_ps(b), r;
    __asm__("vsubps %2, %1, %0" : "=v"(r) : "v"(va), "v"(vb));
    _mm256_storeu_ps(out, r);
}

/* Values that live across the statement, in a loop. */
NOINLINE static void across(int *out, const int *a, const int *b, int rounds)
{
    __m256i acc = _mm256_setzero_si256(), va = load(a), vb = load(b);
    __m256i one = _mm256_set1_epi32(1);
    double scale = 1.5, sum = 0.0;
    for (int i = 0; i < rounds; i++) {
        __m256i t;
        __asm__("vpaddd %2, %1, %0" : "=x"(t) : "x"(va), "x"(vb));
        acc = _mm256_add_epi32(acc, t);
        va = _mm256_add_epi32(va, one);
        sum += scale * i;
    }
    store(out, acc);
    out[0] += (int)sum;
}

/* A register variable: the operand is in the variable's register. */
NOINLINE static void register_variable(int *out, const int *a, const int *b)
{
    register __m256i pinned __asm__("ymm7") = load(a);
    __m256i vb = load(b), r;
    __asm__("vpaddd %%ymm7, %2, %0" : "=x"(r) : "x"(pinned), "x"(vb));
    store(out, r);
}

/* Nine operands: more than the eight registers of 32-bit code. */
#ifdef __x86_64__
NOINLINE static void many(int *out, const int *a)
{
    __m256i v0 = load(a), v1 = load(a + 1), v2 = load(a + 2), v3 = load(a + 3);
    __m256i v4 = load(a + 4), v5 = load(a + 5), v6 = load(a + 6), v7 = load(a + 7);
    __m256i r;
    __asm__("vpaddd %2, %1, %0\n\tvpaddd %3, %0, %0\n\tvpaddd %4, %0, %0\n\t"
            "vpaddd %5, %0, %0\n\tvpaddd %6, %0, %0\n\tvpaddd %7, %0, %0\n\t"
            "vpaddd %8, %0, %0"
            : "=&x"(r)
            : "x"(v0), "x"(v1), "x"(v2), "x"(v3), "x"(v4), "x"(v5), "x"(v6), "x"(v7));
    store(out, r);
}
#endif

int main(void)
{
    if (!__builtin_cpu_supports("avx2")) {
        puts("asm wide vector operands: no AVX2 on this machine");
        return 0;
    }

    int a[16], b[16], got[8], want[8];
    for (int i = 0; i < 16; i++) {
        a[i] = 1000 + 37 * i;
        b[i] = 7 - 3 * i;
    }

    add(got, a, b);
    for (int i = 0; i < 8; i++) want[i] = a[i] + b[i];
    expect("add", got, want, sizeof want);

    tied(got, a, b);
    for (int i = 0; i < 8; i++) want[i] = a[i] - b[i];
    expect("tied", got, want, sizeof want);

    read_write(got, a, b);
    for (int i = 0; i < 8; i++) want[i] = a[i] * b[i];
    expect("read_write", got, want, sizeof want);

    halves(got, a);
    for (int i = 0; i < 4; i++) want[i] = a[i] + a[i + 4];
    for (int i = 4; i < 8; i++) want[i] = 0;
    expect("halves", got, want, sizeof want);

    rvalue(got, a, b);
    for (int i = 0; i < 8; i++) want[i] = (a[i] + b[i]) * 2;
    expect("rvalue", got, want, sizeof want);

    register_or_memory(got, a, b);
    for (int i = 0; i < 8; i++) want[i] = a[i] + b[i];
    expect("register_or_memory", got, want, sizeof want);

    named_register(got, a, b);
    for (int i = 0; i < 8; i++) want[i] = a[i] - b[i];
    expect("named_register", got, want, sizeof want);

    {
        struct box src, dst;
        __m256i pointee = load(b), arr[2];
        src.tag = 1;
        src.v = load(a);
        dst.tag = 2;
        arr[0] = load(a);
        arr[1] = load(a + 8);
        places(&dst, &src, &pointee, arr);
        store(got, dst.v);
        for (int i = 0; i < 8; i++) want[i] = a[i] + b[i] + a[i + 8];
        expect("places", got, want, sizeof want);
        if (dst.tag != 2 || src.tag != 1) {
            puts("FAIL places: a neighbour was written");
            failures++;
        }
    }

    {
        double da[4] = { 1.5, -2.25, 3.0, 1e10 }, db[4] = { 2.0, 4.0, -0.5, 1e-10 };
        double dgot[4], dwant[4];
        doubles(dgot, da, db);
        for (int i = 0; i < 4; i++) dwant[i] = da[i] * db[i];
        expect("doubles", dgot, dwant, sizeof dwant);
    }

    {
        float fa[8], fb[8], fgot[8], fwant[8];
        for (int i = 0; i < 8; i++) {
            fa[i] = 0.5f * (float)i;
            fb[i] = 3.0f - (float)i;
            fwant[i] = fa[i] - fb[i];
        }
        floats(fgot, fa, fb);
        expect("floats", fgot, fwant, sizeof fwant);
    }

    across(got, a, b, 5);
    for (int i = 0; i < 8; i++) want[i] = 5 * (a[i] + b[i]) + 10;
    want[0] += 15;
    expect("across", got, want, sizeof want);

    register_variable(got, a, b);
    for (int i = 0; i < 8; i++) want[i] = a[i] + b[i];
    expect("register_variable", got, want, sizeof want);

#ifdef __x86_64__
    {
        int wide[16];
        for (int i = 0; i < 16; i++) wide[i] = 3 * i + 1;
        many(got, wide);
        for (int i = 0; i < 8; i++) {
            want[i] = 0;
            for (int k = 0; k < 8; k++) want[i] += wide[i + k];
        }
        expect("many", got, want, sizeof want);
    }
#endif

    if (failures == 0)
        puts("asm wide vector operands: PASS");
    return failures != 0;
}
