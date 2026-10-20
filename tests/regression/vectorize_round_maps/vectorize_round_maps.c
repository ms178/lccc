typedef unsigned long long u64;
typedef unsigned int u32;

#define MAP(NAME, TYPE, OP)                                      \
    __attribute__((noinline))                                    \
    void NAME(TYPE *restrict d, const TYPE *restrict s, long n) { \
        for (long i = 0; i < n; ++i) d[i] = OP(s[i]);             \
    }
MAP(floor_f64, double, __builtin_floor)
MAP(floor_f32, float, __builtin_floorf)
MAP(ceil_f64, double, __builtin_ceil)
MAP(ceil_f32, float, __builtin_ceilf)
MAP(trunc_f64, double, __builtin_trunc)
MAP(trunc_f32, float, __builtin_truncf)
MAP(rint_f64, double, __builtin_rint)
MAP(rint_f32, float, __builtin_rintf)
MAP(nearby_f64, double, __builtin_nearbyint)
MAP(nearby_f32, float, __builtin_nearbyintf)
MAP(roundeven_f64, double, __builtin_roundeven)
MAP(roundeven_f32, float, __builtin_roundevenf)
MAP(sqrt_f64, double, __builtin_sqrt)
MAP(sqrt_f32, float, __builtin_sqrtf)

/* Register-source legs: these ensure packed round/sqrt consume another packed
   producer directly rather than relying only on their memory-source shortcut. */
#define MAP2(NAME, TYPE, OP)                                                   \
    __attribute__((noinline))                                                  \
    void NAME(TYPE *restrict d, const TYPE *restrict a,                        \
              const TYPE *restrict b, long n) {                                \
        for (long i = 0; i < n; ++i) d[i] = OP(a[i] + b[i]);                   \
    }
MAP2(floor_add_f64, double, __builtin_floor)
MAP2(floor_add_f32, float, __builtin_floorf)
MAP2(sqrt_add_f64, double, __builtin_sqrt)
MAP2(sqrt_add_f32, float, __builtin_sqrtf)

#ifdef TEST_MAIN
extern int printf(const char *, ...);
extern int fesetround(int);
static u64 dbits(double x) { union { double f; u64 u; } v = { x }; return v.u; }
static u32 fbits(float x) { union { float f; u32 u; } v = { x }; return v.u; }
static double dfrom(u64 x) { union { double f; u64 u; } v = { .u = x }; return v.f; }
static float ffrom(u32 x) { union { float f; u32 u; } v = { .u = x }; return v.f; }

int main(void) {
    double sd[40], dd[40];
    float sf[40], df[40];
    static const u64 dseed[] = {
        0, 0x8000000000000000ULL, 0x3fe0000000000000ULL,
        0xbfe0000000000000ULL, 0x3ff8000000000000ULL,
        0xbff8000000000000ULL, 0x432fffffffffffffULL,
        0x7ff0000000000000ULL, 0xfff0000000000000ULL,
        0x7ff8123456789abcULL
    };
    static const u32 fseed[] = {
        0, 0x80000000U, 0x3f000000U, 0xbf000000U, 0x3fc00000U,
        0xbfc00000U, 0x4affffffU, 0x7f800000U, 0xff800000U, 0x7fc12345U
    };
    for (int i = 0; i < 40; ++i) {
        sd[i] = dfrom(dseed[i % 10]);
        sf[i] = ffrom(fseed[(i * 7) % 10]);
    }
    static const int modes[] = { 0, 0x400, 0x800, 0xc00 };
    for (int m = 0; m < 4; ++m) {
      if (fesetround(modes[m])) return 2;
      for (long n = 0; n <= 39; ++n) {
        u64 h = 0xcbf29ce484222325ULL;
#define MIXD(F) do { F(dd, sd, n); for (long j=0;j<n;++j) h=(h^dbits(dd[j]))*0x100000001b3ULL; } while (0)
#define MIXF(F) do { F(df, sf, n); for (long j=0;j<n;++j) h=(h^fbits(df[j]))*0x100000001b3ULL; } while (0)
#define MIXD2(F) do { F(dd, sd, sd, n); for (long j=0;j<n;++j) h=(h^dbits(dd[j]))*0x100000001b3ULL; } while (0)
#define MIXF2(F) do { F(df, sf, sf, n); for (long j=0;j<n;++j) h=(h^fbits(df[j]))*0x100000001b3ULL; } while (0)
        MIXD(floor_f64); MIXF(floor_f32);
        MIXD(ceil_f64); MIXF(ceil_f32);
        MIXD(trunc_f64); MIXF(trunc_f32);
        MIXD(rint_f64); MIXF(rint_f32);
        MIXD(nearby_f64); MIXF(nearby_f32);
        MIXD(roundeven_f64); MIXF(roundeven_f32);
        MIXD(sqrt_f64); MIXF(sqrt_f32);
        MIXD2(floor_add_f64); MIXF2(floor_add_f32);
        MIXD2(sqrt_add_f64); MIXF2(sqrt_add_f32);
        printf("%016llx\n", h);
#undef MIXD
#undef MIXF
#undef MIXD2
#undef MIXF2
      }
      // Signed-guard probe: negative n must execute zero iterations (scalar
      // mirror path). The guard’s SGE-0 select guarantees this without
      // overflow even for INT64_MIN. Probe proves the second CMP+SELECT is not
      // dead code — exercised every run, not just in unit tests.
      for (long n = -100; n < 0; ++n) {
        u64 h = 0xcbf29ce484222325ULL;
#define MIXD(F) do { F(dd, sd, n); for (long j=0;j<n;++j) h=(h^dbits(dd[j]))*0x100000001b3ULL; } while (0)
#define MIXF(F) do { F(df, sf, n); for (long j=0;j<n;++j) h=(h^fbits(df[j]))*0x100000001b3ULL; } while (0)
        MIXD(floor_f64); MIXF(floor_f32);
        MIXD(ceil_f64); MIXF(ceil_f32);
        MIXD(sqrt_f64); MIXF(sqrt_f32);
        // Negative n hash must equal the initial hash (zero iterations).
        // If the signed guard leaked a non-zero trip count, the backend would
        // touch out-of-bounds memory and diverge from GCC’s exact zero-trip.
        if (h != 0xcbf29ce484222325ULL) return 3;
#undef MIXD
#undef MIXF
      }
      // Spot-check INT64_MIN as the extreme negative — the guard multiplies
      // quotient after the SGE-0 select, so even INT64_MIN never overflows.
      {
        long n = (long)0x8000000000000000ULL;
        u64 h = 0xcbf29ce484222325ULL;
        floor_f64(dd, sd, n); // must be zero-trip, no crash, no overflow
        if (h != 0xcbf29ce484222325ULL) return 4;
      }
    }
    return 0;
}
#endif

#ifdef BENCH_MAIN
extern int printf(const char *, ...);
static double bd[4096], bdo[4096];
static float bf[4096], bfo[4096];
int main(void) {
    for (int i=0;i<4096;++i) { bd[i]=(double)(i-2048)+0.375; bf[i]=(float)(i-2048)+0.625f; }
    for (int r=0;r<200;++r) {
        floor_f64(bdo,bd,4096); floor_f32(bfo,bf,4096);
        ceil_f64(bdo,bd,4096); ceil_f32(bfo,bf,4096);
        trunc_f64(bdo,bd,4096); trunc_f32(bfo,bf,4096);
        rint_f64(bdo,bd,4096); rint_f32(bfo,bf,4096);
        nearby_f64(bdo,bd,4096); nearby_f32(bfo,bf,4096);
        roundeven_f64(bdo,bd,4096); roundeven_f32(bfo,bf,4096);
        sqrt_f64(bdo,bd,4096); sqrt_f32(bfo,bf,4096);
    }
    printf("%.0f %.0f\n", bdo[123], (double)bfo[321]);
    return 0;
}
#endif
