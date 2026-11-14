/* 256-bit load/store intrinsics must be ONE instruction, at every -O level.
 *
 * The bundled <avxintrin.h> implemented `_mm256_loadu_ps`/`storeu_ps`/
 * `loadu_pd`/`storeu_pd` as lane loops (`for (i...) __r[i] = __p[i];`), and
 * `_mm256_loadu_si256`/`storeu_si256`/`lddqu_si256` as `__builtin_memcpy` of a
 * 32-byte object.  Those only became a single `vmovups`/`vmovdqu` once SLP ran:
 * measured at -O0 a load cost 28 instructions and a 176-byte stack frame, a
 * store 22 instructions, where gcc-16 emits two.  The aligned forms
 * (`_mm256_load_ps`, `_mm256_store_ps`, `_mm256_load_pd`, `_mm256_store_pd`)
 * did not exist at all.
 *
 * Each header function now calls the frontend builtin
 * (`__builtin_ia32_loadups256` and friends) that lowers straight to the move,
 * so the intrinsic is one instruction regardless of optimization level.
 *
 * Both `in` and `out` in the show_* helpers are 32-byte aligned: they feed the
 * ALIGNED intrinsics, and a reference compiler emits `vmovaps` for those and
 * faults on a misaligned stack object (gcc-16 segfaulted on the first draft of
 * this fixture, which is how the requirement was found).
 *
 * Structure: the `core_*` functions contain ONLY the intrinsic under test, so
 * the gate can census their instructions without counting printf's argument
 * loads; the `show_*` functions do the formatting.  Every core also writes its
 * result where the program prints it, so the byte parity check covers the same
 * code the shape check counts.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned fbits(float f)
{
    unsigned u;
    memcpy(&u, &f, sizeof(u));
    return u;
}

static unsigned long long dbits(double d)
{
    unsigned long long u;
    memcpy(&u, &d, sizeof(u));
    return u;
}

/* Alignment is what separates the aligned from the unaligned forms; these
 * buffers are 32-byte aligned so both are legal. */
static float fa[8] __attribute__((aligned(32)));
static double da[4] __attribute__((aligned(32)));
static unsigned char ibuf[32] __attribute__((aligned(32)));

/* ── core: one load and one store, nothing else ─────────────────────────── */
static __attribute__((noinline)) void core_u_ps(const float *in, float *out)
{
    _mm256_storeu_ps(out, _mm256_loadu_ps(in));
}

static __attribute__((noinline)) void core_a_ps(const float *in, float *out)
{
    _mm256_store_ps((float *)fa, _mm256_load_ps(in));
    _mm256_store_ps(out, _mm256_load_ps((const float *)fa));
}

static __attribute__((noinline)) void core_u_pd(const double *in, double *out)
{
    _mm256_storeu_pd(out, _mm256_loadu_pd(in));
}

static __attribute__((noinline)) void core_a_pd(const double *in, double *out)
{
    _mm256_store_pd((double *)da, _mm256_load_pd(in));
    _mm256_store_pd(out, _mm256_load_pd((const double *)da));
}

static __attribute__((noinline)) void core_u_si(const void *in, void *out)
{
    _mm256_storeu_si256((__m256i_u *)out, _mm256_loadu_si256((const __m256i_u *)in));
}

static __attribute__((noinline)) void core_lddqu_si(const void *in, void *out)
{
    _mm256_storeu_si256((__m256i_u *)out, _mm256_lddqu_si256((const __m256i_u *)in));
}

/* ── show: build inputs, call the core, print the bytes ─────────────────── */
static void show_ps(const char *tag, void (*core)(const float *, float *))
{
    float in[8] __attribute__((aligned(32)));
    float out[8] __attribute__((aligned(32)));
    for (int i = 0; i < 8; i++)
        in[i] = (float)i * 1.5f - 3.0f;
    memset(out, 0, sizeof(out));
    core(in, out);
    for (int i = 0; i < 8; i++)
        printf("%s %08x\n", tag, fbits(out[i]));
}

static void show_pd(const char *tag, void (*core)(const double *, double *))
{
    double in[4] __attribute__((aligned(32)));
    double out[4] __attribute__((aligned(32)));
    for (int i = 0; i < 4; i++)
        in[i] = (double)i / 3.0 - 1.25;
    memset(out, 0, sizeof(out));
    core(in, out);
    for (int i = 0; i < 4; i++)
        printf("%s %016llx\n", tag, dbits(out[i]));
}

static void show_si(const char *tag, void (*core)(const void *, void *))
{
    unsigned char out[32];
    for (int i = 0; i < 32; i++)
        ibuf[i] = (unsigned char)(i * 7 + 3);
    memset(out, 0, sizeof(out));
    core(ibuf, out);
    printf("%s", tag);
    for (int i = 0; i < 32; i++)
        printf(" %02x", out[i]);
    printf("\n");
}

int main(void)
{
    show_ps("u_ps", core_u_ps);
    show_ps("a_ps", core_a_ps);
    show_pd("u_pd", core_u_pd);
    show_pd("a_pd", core_a_pd);
    show_si("u_si", core_u_si);
    show_si("lddqu_si", core_lddqu_si);
    /* Keep the aligned buffers live for the a_* paths. */
    fa[0] = fa[7];
    da[0] = da[3];
    return (int)(fa[0] + (float)da[0]) & 0;
}
