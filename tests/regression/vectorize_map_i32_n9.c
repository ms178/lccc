
void scale_add(int *restrict dst, const int *restrict src, int scale, int offset, int n) {
    for (int i = 0; i < n; i++)
        dst[i] = src[i] * scale + offset;
}
int main(void) {
    int src[9], dst[9];
    for (int i = 0; i < 9; i++) src[i] = i * i - 3;
    scale_add(dst, src, 11, -5, 9);
    for (int i = 0; i < 9; i++)
        if (dst[i] != src[i] * 11 - 5) return 1 + i;
    return 0;
}
