int sum_i32(const int *restrict a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
int main(void) {
    int a[16];
    int expect = 0;
    for (int i = 0; i < 16; i++) { a[i] = i + 1; expect += a[i]; }
    if (sum_i32(a, 16) != expect) return 1;
    if (sum_i32(a, 0) != 0) return 2;
    return 0;
}
