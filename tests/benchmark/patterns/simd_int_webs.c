/*
 * Integer-SIMD web corpus: loop shapes that stress the destructive SSE-128
 * allocator and the segment-precise FP/vector phi-move check.
 *
 * Every externally-visible noinline function isolates one high-value integer
 * vector shape.  There is deliberately no main(): scripts/godbolt.py and
 * scripts/codegen_oracle.py --rank compare each function against GCC 16.2,
 * Clang, ICC and current ICX without benchmark-harness noise.
 *
 * Run two semantic modes:
 *   strict: -O3 -march=x86-64-v3
 *   fast:   -O3 -march=x86-64-v3 -ffast-math -ffp-contract=fast
 */
#define NOINLINE __attribute__((noinline))
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

/* 01-04: widening/narrow reductions over sub-word lanes. */
NOINLINE u32 i01_sum_u8(const u8 *restrict a, int n) {
    u32 s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
NOINLINE int i02_sum_i16(const short *restrict a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
NOINLINE long long i03_sum_i32(const int *restrict a, int n) {
    long long s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
NOINLINE u64 i04_sum_u16_wide(const u16 *restrict a, int n) {
    u64 s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

/* 05-08: min/max/avg/absdiff webs (unsigned + signed, sub-word). */
NOINLINE u8 i05_min_u8(const u8 *restrict a, int n) {
    u8 m = 255;
    for (int i = 0; i < n; i++) m = a[i] < m ? a[i] : m;
    return m;
}
NOINLINE short i06_max_i16(const short *restrict a, int n) {
    short m = -32768;
    for (int i = 0; i < n; i++) m = a[i] > m ? a[i] : m;
    return m;
}
NOINLINE void i07_avg_u8(u8 *restrict d, const u8 *restrict a,
                         const u8 *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = (u8)(((unsigned)a[i] + b[i] + 1) >> 1);
}
NOINLINE u32 i08_absdiff_sum_u8(const u8 *restrict a, const u8 *restrict b,
                                int n) {
    u32 s = 0;
    for (int i = 0; i < n; i++) {
        unsigned x = a[i], y = b[i];
        s += x > y ? x - y : y - x;
    }
    return s;
}

/* 09-12: saturating arithmetic + pmaddwd + blend + bswap webs. */
NOINLINE void i09_sadd_sat_i16(short *restrict d, const short *restrict a,
                               const short *restrict b, int n) {
    for (int i = 0; i < n; i++) {
        int t = (int)a[i] + b[i];
        if (t > 32767) t = 32767;
        if (t < -32768) t = -32768;
        d[i] = (short)t;
    }
}
NOINLINE int i10_dot_i16(const short *restrict a, const short *restrict b,
                         int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += (int)a[i] * b[i];
    return s;
}
NOINLINE void i11_select_add(int *restrict d, const int *restrict a,
                             const int *restrict b, const int *restrict c,
                             int n) {
    for (int i = 0; i < n; i++) d[i] = (c[i] < 0) ? a[i] + 1 : b[i] + 2;
}
NOINLINE void i12_bswap32(u32 *restrict d, const u32 *restrict a, int n) {
    for (int i = 0; i < n; i++) {
        u32 x = a[i];
        d[i] = (x >> 24) | ((x >> 8) & 0xff00) | ((x << 8) & 0xff0000) |
               (x << 24);
    }
}

/* 13-16: widen/narrow/pack + multi-accumulator webs. */
NOINLINE void i13_widen_add(int *restrict d, const u8 *restrict a,
                            const u8 *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = (int)a[i] + b[i];
}
NOINLINE void i14_pack_clamp(u8 *restrict d, const int *restrict a, int n) {
    for (int i = 0; i < n; i++) {
        int t = a[i];
        if (t > 255) t = 255;
        if (t < 0) t = 0;
        d[i] = (u8)t;
    }
}
NOINLINE long long i15_sum2_i32(const int *restrict a,
                                const int *restrict b, int n) {
    long long s0 = 0, s1 = 0;
    for (int i = 0; i < n; i++) {
        s0 += a[i];
        s1 += b[i];
    }
    return s0 + 2 * s1;
}
NOINLINE long long i16_sum3_i16(const short *restrict a,
                                const short *restrict b,
                                const short *restrict c, int n) {
    long long s0 = 0, s1 = 0, s2 = 0;
    for (int i = 0; i < n; i++) {
        s0 += a[i];
        s1 += b[i];
        s2 += c[i];
    }
    return s0 + s1 + s2;
}

/* 17-20: axpy/cmpeq/mulld/rotate chains (destructive-op heavy). */
NOINLINE void i17_axpy_i32(int *restrict d, const int *restrict a, int k,
                           int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * k + d[i];
}
NOINLINE int i18_count_eq_i32(const int *restrict a, const int *restrict b,
                              int n) {
    int c = 0;
    for (int i = 0; i < n; i++) c += (a[i] == b[i]);
    return c;
}
NOINLINE void i19_mul_lo_i32(int *restrict d, const int *restrict a,
                             const int *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * b[i] + 1;
}
NOINLINE u32 i20_rot_add_u32(const u32 *restrict a, int n) {
    u32 s = 0;
    for (int i = 0; i < n; i++) {
        u32 x = a[i];
        x = (x << 5) | (x >> 27);
        s += x;
    }
    return s;
}
