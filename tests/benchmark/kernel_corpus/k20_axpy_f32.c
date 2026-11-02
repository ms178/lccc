/* SAXPY: multiply-add stream with scalar broadcast. */
void k20_axpy_f32(float *d, const float *a, float k, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * k + d[i];
}
