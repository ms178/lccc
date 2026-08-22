// Adversarial guard for the late AArch64 vectorization pass: a conditional
// sum (`if (x > 0) s += x`) is Select-shaped after if_convert — the same
// surface shape as a max reduction. It must NOT be vectorized as an
// unconditional sum: reduction_pattern_is_sound check (2) rejects the
// Select because it reads the accumulator and is not the identified Add.
// This test pins that structural guarantee with negative-heavy data where
// an unconditional sum would differ wildly.
int printf(const char *, ...);
static int a[4096];
long cond_sum(int n) {
    long s = 0;
    for (int i = 0; i < n; i++) { int x = a[i]; if (x > 0) s += x; }
    return s;
}
int main() {
    for (int i = 0; i < 4096; i++) a[i] = (int)((i * 2654435761u) % 100000) - 50000;
    printf("%ld %ld %ld\n", cond_sum(7), cond_sum(4000), cond_sum(4096));
    return 0;
}
