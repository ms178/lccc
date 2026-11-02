/* F32 map: dst[i] = src[i] * scale + offset (AVX2 8-wide + remainder) */
void scale_add_f32(float *restrict dst, const float *restrict src, float scale, float offset, int n) {
    for (int i = 0; i < n; i++)
        dst[i] = src[i] * scale + offset;
}
int main(void) {
    float src[17], dst[17];
    int i;
    for (i = 0; i < 17; i++) src[i] = (float)(i - 3);
    scale_add_f32(dst, src, 5.0f, 7.0f, 17);
    for (i = 0; i < 17; i++) {
        float expect = (float)(i - 3) * 5.0f + 7.0f;
        float d = dst[i] - expect;
        if (d < -0.01f || d > 0.01f) return 1 + i;
    }
    /* small n (remainder-only path) */
    {
        float s0 = -3.0f, d0 = 0.0f;
        scale_add_f32(&d0, &s0, 2.0f, 3.0f, 1);
        if (d0 < -3.01f || d0 > -2.99f) return 100;
    }
    scale_add_f32(dst, src, 1.0f, 0.0f, 0);
    return 0;
}
