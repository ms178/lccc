// Regression: the call staging-hazard pre-spill area (session 25) moves rsp
// but did not bump rsp_frame_size, so rsp-relative slot reads emitted while
// staging the remaining arguments read hazard_area bytes below their slot.
// Four FP args where one is freshly computed (hazard) and three come from
// slots reproduces it: args 1-3 of printf were stale/garbage.
int printf(const char *, ...);
double s1(double a, double b, double c) { return c - a * b; }
double s2(double a, double b, double c) { return a * b - c; }
float t1(float a, float b, float c) { return c - a * b; }
float t2(float a, float b, float c) { return a * b - c; }
int main(void) {
    volatile double a = 3, b = 4, c = 20;
    volatile float x = 3, y = 4, z = 20;
    printf("%.1f %.1f %.1f %.1f\n", s1(a, b, c), s2(a, b, c),
           (double)t1(x, y, z), (double)t2(x, y, z));
    return 0;
}
