/* AB-14: removing an earlier dead parameter Alloca must not shift the positional
 * home of a later stack-passed long double parameter. */
__attribute__((noinline)) static void wr(long double *p, long double v) { p[1] = v; }
__attribute__((noinline)) static long double rd(const long double *p) { return p[1]; }
int main(void) {
    long double x[2] = { 0.0L, 0.0L };
    wr(x, 7.5L);
    return rd(x) == 7.5L ? 0 : 1;
}
