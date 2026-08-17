
int sum_i32(const int *restrict a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
int main(void) {
    enum { N = 128 };
    int a[N];
    int expect = 0;
    for (int i = 0; i < N; i++) { a[i] = i - 64; expect += a[i]; }
    if (sum_i32(a, N) != expect) return 1;
    if (sum_i32(a, 1) != a[0]) return 2;
    if (sum_i32(a, 0) != 0) return 3;
    return 0;
}
