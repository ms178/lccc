int main(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = (i + 1) * 7;
    int s = 0;
    for (int i = 0; i < 4; i++) s += a[i];
    return s == 70 ? 0 : 1;
}
