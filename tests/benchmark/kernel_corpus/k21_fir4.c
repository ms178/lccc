/* 4-tap FIR: sliding-window dot with overlapping loads. */
void k21_fir4(int *d, const int *a, const int *c, int n) {
    for (int i = 0; i < n; i++)
        d[i] = a[i] * c[0] + a[i + 1] * c[1] + a[i + 2] * c[2] + a[i + 3] * c[3];
}
