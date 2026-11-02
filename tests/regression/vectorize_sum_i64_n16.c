
long long sum_i64(const long long *restrict a, int n) {
    long long s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
int main(void) {
    long long a[16];
    long long expect = 0;
    for (int i = 0; i < 16; i++) { a[i] = i * 1000000007LL - 3; expect += a[i]; }
    if (sum_i64(a, 16) != expect) return 1;
    return 0;
}
