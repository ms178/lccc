/* v4 regression: SSE float ops (ps/pd) vs scalar reference — xor/and/or/add/
 * sub/mul/div/min/max/sqrt/cmp. The bundled headers implement these via
 * __builtin_memcpy fallbacks; this validates the v3 real-intrinsic lowering. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static int feq(float a, float b) { return a == b || (a != a && b != b); }

int main(void) {
    float a[4] = {1.5f, -2.25f, 3.75f, -0.5f};
    float b[4] = {2.0f, 0.5f, -1.25f, 4.0f};
    __m128 va = _mm_loadu_ps(a);
    __m128 vb = _mm_loadu_ps(b);

    float r[4];
    _mm_storeu_ps(r, _mm_add_ps(va, vb));
    for (int i = 0; i < 4; i++) if (!feq(r[i], a[i]+b[i])) return 1;
    _mm_storeu_ps(r, _mm_sub_ps(va, vb));
    for (int i = 0; i < 4; i++) if (!feq(r[i], a[i]-b[i])) return 2;
    _mm_storeu_ps(r, _mm_mul_ps(va, vb));
    for (int i = 0; i < 4; i++) if (!feq(r[i], a[i]*b[i])) return 3;
    _mm_storeu_ps(r, _mm_div_ps(va, vb));
    for (int i = 0; i < 4; i++) if (!feq(r[i], a[i]/b[i])) return 4;

    /* bitwise: xor/and/or — verify against exact bit patterns */
    uint32_t xa[4] = {0xDEADBEEF, 0x12345678, 0xFFFFFFFF, 0x00000000};
    uint32_t xb[4] = {0xCAFEBABE, 0x0F0F0F0F, 0xAAAAAAAA, 0x55555555};
    __m128 vxa = _mm_loadu_ps((const float*)xa);
    __m128 vxb = _mm_loadu_ps((const float*)xb);
    uint32_t xr[4];
    _mm_storeu_ps((float*)xr, _mm_xor_ps(vxa, vxb));
    for (int i = 0; i < 4; i++) if (xr[i] != (xa[i] ^ xb[i])) return 5;
    _mm_storeu_ps((float*)xr, _mm_and_ps(vxa, vxb));
    for (int i = 0; i < 4; i++) if (xr[i] != (xa[i] & xb[i])) return 6;
    _mm_storeu_ps((float*)xr, _mm_or_ps(vxa, vxb));
    for (int i = 0; i < 4; i++) if (xr[i] != (xa[i] | xb[i])) return 7;
    _mm_storeu_ps((float*)xr, _mm_andnot_ps(vxa, vxb));
    for (int i = 0; i < 4; i++) if (xr[i] != (~xa[i] & xb[i])) return 8;

    /* min/max/sqrt */
    float m[4];
    _mm_storeu_ps(m, _mm_min_ps(va, vb));
    for (int i = 0; i < 4; i++) if (m[i] != (a[i] < b[i] ? a[i] : b[i])) return 9;
    _mm_storeu_ps(m, _mm_max_ps(va, vb));
    for (int i = 0; i < 4; i++) if (m[i] != (a[i] > b[i] ? a[i] : b[i])) return 10;
    float pos[4] = {1.0f, 4.0f, 9.0f, 16.0f};
    _mm_storeu_ps(m, _mm_sqrt_ps(_mm_loadu_ps(pos)));
    for (int i = 0; i < 4; i++) if (m[i] != pos[i] / (i == 0 ? 1.0f : (i==1?2.0f:(i==2?3.0f:4.0f)))) return 11;

    /* comparisons -> movemask */
    int cm = _mm_movemask_ps(_mm_cmplt_ps(va, vb));
    for (int i = 0; i < 4; i++) if (((cm >> i) & 1) != (a[i] < b[i] ? 1 : 0)) return 12;

    /* doubles */
    double da[2] = {1.5, -2.25};
    double db[2] = {0.5, 3.0};
    __m128d vda = _mm_loadu_pd(da);
    __m128d vdb = _mm_loadu_pd(db);
    double dr[2];
    _mm_storeu_pd(dr, _mm_add_pd(vda, vdb));
    if (dr[0] != 2.0 || dr[1] != 0.75) return 13;
    _mm_storeu_pd(dr, _mm_mul_pd(vda, vdb));
    if (dr[0] != 0.75 || dr[1] != -6.75) return 14;

    /* scalar single */
    if (_mm_cvtss_f32(_mm_add_ss(_mm_set_ss(1.5f), _mm_set_ss(2.5f))) != 4.0f) return 15;

    /* horizontal adds */
    _mm_storeu_ps(m, _mm_hadd_ps(va, vb));   /* SSSE3 */
    if (!feq(m[0], a[0]+a[1])) return 16;
    if (!feq(m[1], a[2]+a[3])) return 17;

    /* cast round trips (v3 CastReinterpret) */
    __m128i as_i = _mm_castps_si128(va);
    __m128 as_f = _mm_castsi128_ps(as_i);
    _mm_storeu_ps(m, as_f);
    for (int i = 0; i < 4; i++) if (m[i] != a[i]) return 18;

    printf("OK simd_sse_float\n");
    return 0;
}
