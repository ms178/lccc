
float sum_f32(const float *restrict a, int n) {
    float s = 0.0f;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
int main(void) {
    float a[16];
    float expect = 0.0f;
    for (int i = 0; i < 16; i++) { a[i] = (float)(i + 1); expect += a[i]; }
    float s = sum_f32(a, 16);
    if (s < expect - 0.1f || s > expect + 0.1f) return 1;
    if (sum_f32(a, 0) != 0.0f) return 2;
    return 0;
}
