/* FP Backedge PRE profitability guard: default FP PRE is allowed only for
 * multi-use top expressions, avoiding the Mandelbrot singly-used-square
 * regression while still capturing a measured FP win.
 */
extern void abort(void);
__attribute__((noinline)) double run(unsigned long n) {
    double x = 1.000001, acc = 0.0;
    for (unsigned long i = 0; i < n; ++i) {
        double y = x * x;
        acc += y * 0.25 + y * 0.125;
        x += 0.000001;
        double z = x * x;
        acc += z * 0.03125;
    }
    return acc + x;
}
int main(void) {
    double r = run(1000);
    if (r < 407.6578 || r > 407.6579) abort();
    return 0;
}
