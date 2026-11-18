/* Regression: the 256-bit FLOAT vector families must be usable under -mavx
 * alone (AVX1, no AVX2). A scalar broadcast into an 8 x f32 or 4 x f64 vector
 * literal, the arithmetic on it, and the lane loads/stores are all AVX1
 * instructions (vbroadcastss/vbroadcastsd, vmovups, vmulps, vaddps, ...).
 * Before the fix the vectorizer only formed these shapes under AVX2, so every
 * such value round-tripped through the stack.
 *
 * Each vector result is checked against the same IEEE operations written lane
 * by lane in scalar code, over ordinary values plus signed zeros, infinities,
 * NaNs, a subnormal and the extreme finite values. Every non-NaN result must
 * match bit for bit. A NaN result matches any NaN: IEEE 754 leaves the sign and
 * payload of an arithmetic NaN implementation-defined (GCC 16.2 and lccc differ
 * on `-x + 0.0f` for a NaN input), so NaNs are canonicalized before comparing
 * and hashing. The program exits nonzero on any mismatch, so a wrong answer
 * cannot pass by printing a plausible checksum. Compiled with -ffp-contract=off
 * so no compiler fuses the multiply and add (the reference and the vector code
 * must round identically). */
#include <stdio.h>
#include <string.h>

typedef float v8sf __attribute__((vector_size(32)));
typedef double v4df __attribute__((vector_size(32)));

static v8sf set1_ps(float w) {
    return (v8sf){w, w, w, w, w, w, w, w};
}

static v4df set1_pd(double w) {
    return (v4df){w, w, w, w};
}

/* y[i] = y[i] + a * x[i] over 8-lane groups; a is broadcast once per call. */
__attribute__((noinline)) static void axpy8(float *y, const float *x, float a, int n) {
    for (int i = 0; i + 8 <= n; i += 8) {
        v8sf vx, vy;
        memcpy(&vx, x + i, sizeof vx);
        memcpy(&vy, y + i, sizeof vy);
        v8sf r = vy + set1_ps(a) * vx;
        memcpy(y + i, &r, sizeof r);
    }
}

/* Same shape in double, 4 lanes, with a subtract and a divide. */
__attribute__((noinline)) static void mad4(double *y, const double *x, double a, int n) {
    for (int i = 0; i + 4 <= n; i += 4) {
        v4df vx, vy;
        memcpy(&vx, x + i, sizeof vx);
        memcpy(&vy, y + i, sizeof vy);
        v4df r = (vy - set1_pd(a)) / (vx + set1_pd(1.0)) + set1_pd(a) * vx;
        memcpy(y + i, &r, sizeof r);
    }
}

/* A scalar read through a pointer splatted into eight lanes, then multiplied.
 * The splat must be one memory-source broadcast (vbroadcastss (%reg)), not
 * eight scalar stores and a 32-byte reload. */
__attribute__((noinline)) static void scale8_mem(float *y, const float *x, const float *p) {
    v8sf s = {*p, *p, *p, *p, *p, *p, *p, *p};
    v8sf v;
    memcpy(&v, x, sizeof v);
    v = v * s;
    memcpy(y, &v, sizeof v);
}

/* Negation (a sign-bit flip) and a lane extract, in one kernel. */
__attribute__((noinline)) static void neg8(float *y, const float *x, int n) {
    for (int i = 0; i + 8 <= n; i += 8) {
        v8sf vx;
        memcpy(&vx, x + i, sizeof vx);
        v8sf r = -vx + set1_ps(0.0f);
        memcpy(y + i, &r, sizeof r);
    }
}

static unsigned long long fnv(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 1099511628211ull;
}

/* Bit pattern with every NaN folded to one canonical quiet NaN. */
static unsigned long long bits_f(float f) {
    unsigned u;
    if (f != f) return 0x7fc00000u;
    memcpy(&u, &f, sizeof u);
    return u;
}

static unsigned long long bits_d(double d) {
    unsigned long long u;
    if (d != d) return 0x7ff8000000000000ull;
    memcpy(&u, &d, sizeof u);
    return u;
}

/* A fixed mix of special values and a deterministic pseudo-random tail. */
static void fill_f(float *a, int n, unsigned *seed) {
    static const unsigned bits[] = {
        0x00000000u, 0x80000000u, 0x7f800000u, 0xff800000u, 0x7fc00001u,
        0x00000001u, 0x007fffffu, 0x7f7fffffu, 0xff7fffffu, 0x3f800000u,
    };
    for (int i = 0; i < n; i++) {
        if (i < (int)(sizeof bits / sizeof bits[0])) {
            memcpy(&a[i], &bits[i], sizeof a[i]);
        } else {
            *seed = *seed * 1664525u + 1013904223u;
            a[i] = (float)((int)(*seed >> 8) - (1 << 23)) / 65536.0f;
        }
    }
}

static void fill_d(double *a, int n, unsigned *seed) {
    static const unsigned long long bits[] = {
        0x0000000000000000ull, 0x8000000000000000ull, 0x7ff0000000000000ull,
        0xfff0000000000000ull, 0x7ff8000000000001ull, 0x0000000000000001ull,
        0x000fffffffffffffull, 0x7fefffffffffffffull, 0xffefffffffffffffull,
        0x3ff0000000000000ull,
    };
    for (int i = 0; i < n; i++) {
        if (i < (int)(sizeof bits / sizeof bits[0])) {
            memcpy(&a[i], &bits[i], sizeof a[i]);
        } else {
            *seed = *seed * 1664525u + 1013904223u;
            a[i] = (double)((int)(*seed >> 8) - (1 << 23)) / 4096.0;
        }
    }
}

int main(void) {
    enum { N = 64 };
    float xf[N], yf[N], yr[N], nf[N], nr[N];
    double xd[N], yd[N], yr2[N];
    unsigned seed = 12345u;
    int bad = 0;
    unsigned long long h = 1469598103934665603ull;

    /* Single-precision: axpy and negation against a scalar reference. */
    fill_f(xf, N, &seed);
    fill_f(yf, N, &seed);
    memcpy(yr, yf, sizeof yr);
    axpy8(yf, xf, 0.5f, N);
    for (int i = 0; i < N; i++) yr[i] = yr[i] + 0.5f * xf[i];
    for (int i = 0; i < N; i++) {
        if (bits_f(yf[i]) != bits_f(yr[i])) bad = 1;
        h = fnv(h, bits_f(yf[i]));
    }

    neg8(nf, xf, N);
    for (int i = 0; i < N; i++) nr[i] = -xf[i] + 0.0f;
    for (int i = 0; i < N; i++) {
        if (bits_f(nf[i]) != bits_f(nr[i])) bad = 1;
        h = fnv(h, bits_f(nf[i]));
    }

    /* Pointer-sourced splat: each lane must equal x[i] * *p exactly. */
    {
        float scale = -0.375f, sc[N];
        for (int i = 0; i + 8 <= N; i += 8) scale8_mem(sc + i, xf + i, &scale);
        for (int i = 0; i + 8 <= N; i++) {
            if (bits_f(sc[i]) != bits_f(xf[i] * scale)) bad = 1;
            h = fnv(h, bits_f(sc[i]));
        }
    }

    /* Double-precision: subtract, divide, add and multiply in 4 lanes. */
    fill_d(xd, N, &seed);
    fill_d(yd, N, &seed);
    memcpy(yr2, yd, sizeof yr2);
    mad4(yd, xd, 0.25, N);
    for (int i = 0; i < N; i++) {
        yr2[i] = (yr2[i] - 0.25) / (xd[i] + 1.0) + 0.25 * xd[i];
    }
    for (int i = 0; i < N; i++) {
        if (bits_d(yd[i]) != bits_d(yr2[i])) bad = 1;
        h = fnv(h, bits_d(yd[i]));
    }

    /* The broadcast constructors themselves, lane by lane. */
    {
        v8sf s = set1_ps(-2.5f);
        v4df t = set1_pd(7.125);
        for (int k = 0; k < 8; k++) {
            if (bits_f(s[k]) != bits_f(-2.5f)) bad = 1;
            h = fnv(h, bits_f(s[k]));
        }
        for (int k = 0; k < 4; k++) {
            if (bits_d(t[k]) != bits_d(7.125)) bad = 1;
            h = fnv(h, bits_d(t[k]));
        }
    }

    printf("avx1_float256_families %016llx %s\n", h, bad ? "FAIL" : "ok");
    return bad;
}
