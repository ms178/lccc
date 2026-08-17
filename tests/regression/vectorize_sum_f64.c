double sum_f64(const double *restrict a, int n) {
    double s = 0.0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}
int main(void) {
    double a[8] = {1,2,3,4,5,6,7,8};
    double s = sum_f64(a, 8);
    if (s < 35.9 || s > 36.1) return 1;
    return 0;
}
