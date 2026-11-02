/* Argmin over floats: FP compare web with integer position carry. */
int k19_minpos_f32(const float *a, int n) {
    int p = 0;
    for (int i = 1; i < n; i++)
        if (a[i] < a[p]) p = i;
    return p;
}
