/* aggcopy16 round-trip forwarding and carrier collapse: every shape the
 * pass rewrites (and the one it must refuse) behaves bit-identically to
 * the GCC oracle.
 *
 *  - h2/hf/hd: `return f()` for __m128i/__m128/__m128d — the carrier
 *    split/reload collapses to a direct return (tail call).
 *  - h3: `_mm_add_epi32(f(p), w)` — the reload feeds a vector intrinsic,
 *    NOT a return; the collapse must fail closed (substituting the I128
 *    carrier into an intrinsic operand once emitted a read of an
 *    uninitialized register: `movdqa %xmm15, %xmm0`).
 *  - h7: `use4(_mm_set1_ps(x))[0]` — a broadcast staged through a private
 *    slot before the call is forwarded register-to-register.
 *  - h0: a dead zero-prefill of the result home must not change the value.
 *  - latest-store-wins: two stores to one slot before the reload.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) __m128i mk4(const int *p)
{
    __m128i v;
    int lanes[4] = { p[0] + 1, p[1] + 2, p[2] + 3, p[3] + 4 };
    memcpy(&v, lanes, 16);
    return v;
}

__attribute__((noinline)) __m128 mkf(const float *p)
{
    __m128 v;
    float lanes[4] = { p[0] * 2.0f, p[1] * 3.0f, p[2] * 4.0f, p[3] * 5.0f };
    memcpy(&v, lanes, 16);
    return v;
}

__attribute__((noinline)) __m128d mkd(const double *p)
{
    __m128d v;
    double lanes[2] = { p[0] + 0.5, p[1] - 0.5 };
    memcpy(&v, lanes, 16);
    return v;
}

__attribute__((noinline)) __m128 use4(__m128 v)
{
    float in[4], out[4];
    __m128 r;
    memcpy(in, &v, 16);
    out[0] = in[0] + 1.0f;
    out[1] = in[1] + 2.0f;
    out[2] = in[2] + 3.0f;
    out[3] = in[3] + 4.0f;
    memcpy(&r, out, 16);
    return r;
}

__attribute__((noinline)) __m128i h2(const int *p) { return mk4(p); }

__attribute__((noinline)) __m128i h3(const int *p, __m128i w)
{
    return _mm_add_epi32(mk4(p), w);
}

__attribute__((noinline)) float h7(float x)
{
    return use4(_mm_set1_ps(x))[0];
}

__attribute__((noinline)) __m128 hf(const float *p) { return mkf(p); }

__attribute__((noinline)) __m128d hd(const double *p) { return mkd(p); }

__attribute__((noinline)) __m128i h0(const int *p)
{
    __m128i v = _mm_setzero_si128();
    v = mk4(p);
    return v;
}

/* Latest store wins: the second set1 overwrites the slot the call reads. */
__attribute__((noinline)) float h8(float x, float y)
{
    __m128 slot = _mm_set1_ps(x);
    slot = _mm_set1_ps(y);
    return use4(slot)[3];
}

static void dump4i(const char *n, __m128i v)
{
    int o[4];
    memcpy(o, &v, 16);
    printf("%s %d %d %d %d\n", n, o[0], o[1], o[2], o[3]);
}

int main(void)
{
    int si[4] = { 100, 200, 300, 400 };
    float sf[4] = { 1.5f, 2.5f, 3.5f, 4.5f };
    double sd[2] = { 10.25, 20.75 };
    __m128i w;
    int wl[4] = { 7, 8, 9, 10 };
    float of[4];
    double od[2];

    memcpy(&w, wl, 16);
    dump4i("h2", h2(si));
    dump4i("h3", h3(si, w));
    printf("h7 %.3f\n", (double)h7(6.25f));
    _mm_storeu_ps(of, hf(sf));
    printf("hf %.3f %.3f %.3f %.3f\n", (double)of[0], (double)of[1],
           (double)of[2], (double)of[3]);
    _mm_storeu_pd(od, hd(sd));
    printf("hd %.3f %.3f\n", od[0], od[1]);
    dump4i("h0", h0(si));
    printf("h8 %.3f\n", (double)h8(-3.0f, 11.5f));

    /* Hard assertions so the test fails standalone, not only vs the oracle. */
    {
        int o[4];
        __m128i r = h3(si, w);
        memcpy(o, &r, 16);
        if (o[0] != 108 || o[1] != 210 || o[2] != 312 || o[3] != 414)
            return 1;
        if (h7(6.25f) != 7.25f)
            return 2;
        if (h8(-3.0f, 11.5f) != 15.5f)
            return 3;
    }
    return 0;
}
