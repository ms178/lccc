void scale_add_f32(float *restrict dst, const float *restrict src, float scale, float offset, int n) {
    for (int i = 0; i < n; i++) dst[i] = src[i] * scale + offset;
}
int main(void) {
    float src[23], dst[23];
    for (int i = 0; i < 23; i++) src[i] = (float)(i - 2);
    scale_add_f32(dst, src, 1.25f, -0.25f, 23);
    for (int i = 0; i < 23; i++) {
        float e = (float)(i - 2) * 1.25f - 0.25f;
        float d = dst[i] - e;
        if (d < -0.05f || d > 0.05f) return 1 + i;
    }
    return 0;
}
