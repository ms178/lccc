
static int Gsrc[16];
static int Gdst[16];
void scale_global(int scale, int offset, int n) {
    for (int i = 0; i < n; i++)
        Gdst[i] = Gsrc[i] * scale + offset;
}
int main(void) {
    for (int i = 0; i < 16; i++) Gsrc[i] = i;
    scale_global(4, 1, 16);
    for (int i = 0; i < 16; i++)
        if (Gdst[i] != i * 4 + 1) return 1 + i;
    return 0;
}
