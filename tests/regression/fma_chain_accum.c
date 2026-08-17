double dot4(const double *a, const double *b) {
    double s = 0.0;
    for (int i = 0; i < 4; i++) s = s + a[i] * b[i];
    return s;
}
int main(void) {
    double a[4] = {1,2,3,4}, b[4] = {5,6,7,8};
    double s = dot4(a, b);
    if (s < 69.9 || s > 70.1) return 1;
    return 0;
}
