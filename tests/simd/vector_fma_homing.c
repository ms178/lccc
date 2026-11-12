/* Register homing of the explicit 256-bit FMA3 map family.
 *
 * Before this fixture existed, `_mm256_fmadd_ps(a,b,c)` on register-resident
 * operands compiled to 28 instructions with a 408-byte frame: the `m3` table
 * entry (`FmaPs132v256`) takes an ADDRESS for each operand and writes through
 * a `dest_ptr`, so every parameter and the result were materialised in
 * 32-byte dynamically-realign'd frame slots and reloaded.  GCC 16.2 emits one
 * `vfmadd132ps`.  Measured on the fastbuild tree of this session:
 *
 *     __m256  f(a,b,c){ return _mm256_fmadd_ps(a,b,c); }     28 -> 2
 *     __m256d f(a,b,c){ return _mm256_fmadd_pd(a,b,c); }     28 -> 2
 *     _mm256_fmadd_ps(_mm256_add_ps(x,y), b, c)              28 -> 3
 *
 * (instruction count between the function label and `ret`; GCC 16.2 emits 2,
 * 2 and 3 for the same three shapes, so the register-form path is at parity.)
 *
 * The count is the point of the gate, but this file also EXECUTES: the
 * register-form path is a different emitter (`emit_avx_map_fma_inner`) than
 * the memory form, and FMA3 is a destructive encoding, so an operand that
 * aliases the destination is only observable at runtime.  Every lane is
 * compared bit-exactly against `fmaf`/`fma` from libm, which is built by a
 * third-party compiler: that makes this a differential, not a self-check.
 */
#include <math.h>
#include <stdio.h>
#include <string.h>
#include <immintrin.h>

/* --- the shapes the gate counts ----------------------------------------- */

__attribute__((noinline)) __m256 fma_ps(__m256 a, __m256 b, __m256 c) {
    return _mm256_fmadd_ps(a, b, c);
}

__attribute__((noinline)) __m256d fma_pd(__m256d a, __m256d b, __m256d c) {
    return _mm256_fmadd_pd(a, b, c);
}

/* Register-valued (rvalue) operands: the product has no address, so the
   legacy memory form had nothing to fold and paid the full round trip. */
__attribute__((noinline)) __m256 fma_nested(__m256 x, __m256 y, __m256 b) {
    return _mm256_fmadd_ps(_mm256_add_ps(x, y), b, x);
}

/* A chain of dependent FMAs: each step's result is the next step's
   accumulator, so a home round trip lands on the carried dependency chain. */
__attribute__((noinline)) __m256 fma_chain(__m256 x, __m256 y, __m256 z) {
    __m256 s = _mm256_fmadd_ps(x, y, z);
    s = _mm256_fmadd_ps(s, x, y);
    s = _mm256_fmadd_ps(s, y, z);
    return s;
}

/* Memory-operand shapes.  `_mm256_loadu_ps` lowers to a pointer-result
   intrinsic (`LoaduPs256` writes 32 bytes to a scratch home) plus a
   register read of that home, so the FMA used to consume the *home*: a
   32-byte frame, a spill and a reload per call (8-9 instructions) where
   GCC 16.2 folds the address into the FMA's memory operand.  The read is
   now re-pointed at the load's source when it is a whole-register copy of
   exactly the bytes the load wrote, which is what these four functions
   pin.  `fma_load_c` cannot fold (an FMA3 memory operand is a multiplicand,
   never the addend), so its budget allows the staging `vmovups`. */
__attribute__((noinline)) __m256 fma_load_a(const float *p, __m256 b, __m256 c) {
    return _mm256_fmadd_ps(_mm256_loadu_ps(p), b, c);
}

__attribute__((noinline)) __m256 fma_load_b(__m256 a, const float *p, __m256 c) {
    return _mm256_fmadd_ps(a, _mm256_loadu_ps(p), c);
}

__attribute__((noinline)) __m256 fma_load_c(__m256 a, __m256 b, const float *p) {
    return _mm256_fmadd_ps(a, b, _mm256_loadu_ps(p));
}

__attribute__((noinline)) __m256d fma_load_pd(const double *p, __m256d b, __m256d c) {
    return _mm256_fmadd_pd(_mm256_loadu_pd(p), b, c);
}

/* The same shape without an FMA: a bare direct load must be one register
   read, not store-to-home + reload. */
__attribute__((noinline)) __m256 load_only(const float *p) {
    return _mm256_loadu_ps(p);
}

/* --- value checks ------------------------------------------------------- */

static int bad;

static void check_ps(const char *name, __m256 got, __m256 a, __m256 b, __m256 c) {
    for (int i = 0; i < 8; i++) {
        float want = fmaf(a[i], b[i], c[i]);
        if (memcmp(&got[i], &want, sizeof want) != 0) {
            printf("  %s lane %d: got %.9g want %.9g\n", name, i, got[i], want);
            bad++;
        }
    }
}

static void check_pd(const char *name, __m256d got, __m256d a, __m256d b, __m256d c) {
    for (int i = 0; i < 4; i++) {
        double want = fma(a[i], b[i], c[i]);
        if (memcmp(&got[i], &want, sizeof want) != 0) {
            printf("  %s lane %d: got %.17g want %.17g\n", name, i, got[i], want);
            bad++;
        }
    }
}

int main(void) {
    /* Values that separate a fused from an unfused product, signed zeros, and
       both ends of the exponent range. */
    __m256 a = {1.5f, -2.25f, 3e30f, -4e-30f, 0.0f, -0.0f, 12345.678f, -1.0f};
    __m256 b = {-2.0f, 4.0f, 1e-30f, 3e30f, -1.0f, 2.0f, 0.5f, 7.0f};
    __m256 c = {0.25f, -0.5f, 1.0f, 2.0f, 3.0f, -4.0f, 100.0f, 0.0f};
    __m256d d = {1.5, -2.25, 3e300, -4e-300};
    __m256d e = {-2.0, 4.0, 1e-300, 3e300};
    __m256d f = {0.25, -0.5, 1.0, 2.0};

    check_ps("fmadd_ps", fma_ps(a, b, c), a, b, c);
    check_pd("fmadd_pd", fma_pd(d, e, f), d, e, f);
    check_ps("fmadd_nested", fma_nested(a, b, c), a + b, c, a);

    /* The chain, against the same chain written with the scalar fma. */
    __m256 got = fma_chain(a, b, c);
    int chain_bad = 0;
    for (int i = 0; i < 8; i++) {
        float s = fmaf(a[i], b[i], c[i]);
        s = fmaf(s, a[i], b[i]);
        s = fmaf(s, b[i], c[i]);
        if (memcmp(&got[i], &s, sizeof s) != 0) {
            printf("  fmadd_chain lane %d: got %.9g want %.9g\n", i, got[i], s);
            chain_bad++;
        }
    }
    bad += chain_bad;

    /* Memory-operand shapes: same values, but now the operands arrive from
       memory instead of registers. */
    /* Each array holds a DIFFERENT operand: a memory operand that silently
       replaced a register one (or a swapped multiplicand/addend) is exactly
       what an aliased test vector would hide. */
    float pa[8];
    float pb[8];
    float pc[8];
    double qd[4];
    double qe[4];
    double qf[4];
    for (int i = 0; i < 8; i++) {
        pa[i] = a[i];
        pb[i] = b[i];
        pc[i] = c[i];
    }
    for (int i = 0; i < 4; i++) {
        qd[i] = d[i];
        qe[i] = e[i];
        qf[i] = f[i];
    }
    check_ps("fmadd_load_a", fma_load_a(pa, b, c), a, b, c);
    check_ps("fmadd_load_b", fma_load_b(a, pb, c), a, b, c);
    check_ps("fmadd_load_c", fma_load_c(a, b, pc), a, b, c);
    check_pd("fmadd_load_pd", fma_load_pd(qd, e, f), d, e, f);

    __m256 loaded = load_only(pb);
    for (int i = 0; i < 8; i++) {
        if (memcmp(&loaded[i], &b[i], sizeof loaded[i]) != 0) {
            printf("  load_only lane %d: got %.9g want %.9g\n", i, loaded[i], b[i]);
            bad++;
        }
    }

    if (bad) {
        printf("FAIL: %d lane(s) disagree with libm fma/fmaf\n", bad);
        return 1;
    }
    printf("PASS: every 256-bit FMA shape agrees bit-exactly with libm\n");
    return 0;
}
