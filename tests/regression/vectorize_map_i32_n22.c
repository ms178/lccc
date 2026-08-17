void scale_add(int *restrict dst, const int *restrict src, int scale, int offset, int n) {
    for (int i = 0; i < n; i++) dst[i] = src[i] * scale + offset;
}
int main(void) {
    int src[22], dst[22];
    for (int i = 0; i < 22; i++) src[i] = i * 3 - 1;
    scale_add(dst, src, 5, 2, 22);
    for (int i = 0; i < 22; i++)
        if (dst[i] != src[i] * 5 + 2) return 1 + i;
    return 0;
}
