void scale_add(int *restrict dst, const int *restrict src, int scale, int offset, int n) {
    for (int i = 0; i < n; i++)
        dst[i] = src[i] * scale + offset;
}
int main(void) {
    int src[17], dst[17];
    for (int i = 0; i < 17; i++) src[i] = i - 3;
    scale_add(dst, src, 5, 7, 17);
    for (int i = 0; i < 17; i++)
        if (dst[i] != (i - 3) * 5 + 7) return 1 + i;
    scale_add(dst, src, 2, 3, 1);
    if (dst[0] != (0 - 3) * 2 + 3) return 100;
    scale_add(dst, src, 1, 0, 0);
    return 0;
}
