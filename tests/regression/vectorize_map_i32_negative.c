
void scale_add(int *restrict dst, const int *restrict src, int scale, int offset, int n) {
    for (int i = 0; i < n; i++)
        dst[i] = src[i] * scale + offset;
}
int main(void) {
    int src[8] = {-1000000, -1, 0, 1, 2, 100, -50, 999};
    int dst[8];
    scale_add(dst, src, -3, 10, 8);
    for (int i = 0; i < 8; i++)
        if (dst[i] != src[i] * -3 + 10) return 1 + i;
    return 0;
}
