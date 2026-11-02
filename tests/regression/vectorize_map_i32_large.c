
void scale_add(int *restrict dst, const int *restrict src, int scale, int offset, int n) {
    for (int i = 0; i < n; i++)
        dst[i] = src[i] * scale + offset;
}
int main(void) {
    enum { N = 64 };
    int src[N], dst[N];
    for (int i = 0; i < N; i++) src[i] = i * 3 - 10;
    scale_add(dst, src, 7, -2, N);
    for (int i = 0; i < N; i++)
        if (dst[i] != src[i] * 7 - 2) return 1 + i;
    return 0;
}
