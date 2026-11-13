/* 512-bit load/store intrinsics: present at all, and one move each.
 *
 * `_mm512_loadu_ps`/`_mm512_storeu_ps` were 16-iteration lane loops that the
 * SLP vectorizer never folded even at -O2 (measured: 39 instructions and a
 * 472-byte frame for a load, 33 for a store, where gcc-16 emits two), and
 * `_mm512_loadu_pd`/`_mm512_storeu_pd`/`_mm512_load_pd`/`_mm512_store_pd`
 * did not exist.  They now route through the same `vmovdqu64` helper the
 * integer `_mm512_loadu_si512` already used.
 *
 * The program self-skips when the host has no AVX-512F, so the differential
 * harness can run it everywhere; the instruction-shape half of the proof is a
 * static asm check in check_wide_load_store.sh and needs no capable CPU.
 *
 * NOTE: lccc still materialises 512-bit vector *values* in memory (the
 * register allocator has no zmm class), so a 512-bit op costs a spill plus a
 * reload around the instruction.  These load/store forms are single moves; the
 * spill traffic is a separate, documented backend limitation and is not
 * hidden by this test.
 */
#include <immintrin.h>
#include <stdio.h>
#include <stdlib.h>
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

static int host_has_avx512f(void)
{
    FILE *f = fopen("/proc/cpuinfo", "r");
    char line[4096];
    int found = 0;
    if (!f)
        return 0;
    while (fgets(line, sizeof(line), f)) {
        if (strncmp(line, "flags", 5) != 0)
            continue;
        if (strstr(line, " avx512f"))
            found = 1;
        break;
    }
    fclose(f);
    return found;
}

static __attribute__((noinline)) void u_ps(void)
{
    float out[16], back[16];
    for (int i = 0; i < 16; i++)
        out[i] = (float)i * 0.75f - 2.0f;
    __m512 v = _mm512_loadu_ps(out);
    _mm512_storeu_ps(back, v);
    printf("512 u_ps");
    for (int i = 0; i < 16; i++)
        printf(" %08x", fbits(back[i]));
    printf("\n");
}

static __attribute__((noinline)) void u_pd(void)
{
    double out[8], back[8];
    for (int i = 0; i < 8; i++)
        out[i] = (double)i / 7.0;
    __m512d v = _mm512_loadu_pd(out);
    _mm512_storeu_pd(back, v);
    printf("512 u_pd");
    for (int i = 0; i < 8; i++)
        printf(" %016llx", dbits(back[i]));
    printf("\n");
}

static float af[16] __attribute__((aligned(64)));
static double ad[8] __attribute__((aligned(64)));

static __attribute__((noinline)) void a_ps(void)
{
    for (int i = 0; i < 16; i++)
        af[i] = (float)(i * 3) + 0.25f;
    __m512 v = _mm512_load_ps(af);
    _mm512_store_ps(af, v);
    printf("512 a_ps");
    for (int i = 0; i < 16; i++)
        printf(" %08x", fbits(af[i]));
    printf("\n");
}

static __attribute__((noinline)) void a_pd(void)
{
    for (int i = 0; i < 8; i++)
        ad[i] = (double)(i * 5) - 0.5;
    __m512d v = _mm512_load_pd(ad);
    _mm512_store_pd(ad, v);
    printf("512 a_pd");
    for (int i = 0; i < 8; i++)
        printf(" %016llx", dbits(ad[i]));
    printf("\n");
}

int main(void)
{
    if (!host_has_avx512f()) {
        printf("skip: host lacks avx512f\n");
        return 0;
    }
    u_ps();
    u_pd();
    a_ps();
    a_pd();
    return 0;
}
