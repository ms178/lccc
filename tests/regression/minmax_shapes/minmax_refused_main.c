/*
 * MINMAX-1 contract 3: the shapes the transform REFUSES must stay correct.
 *
 * `minmax_shapes.c` holds them; this main drives each one over the same
 * hostile inputs the differential harness uses (negative values, INT_MIN /
 * INT_MAX, duplicates, lengths on both sides of the vector width) and prints
 * a checksum.  `check_minmax_reduction.sh` compares lccc's checksum against
 * the GCC oracle's for the same source.
 *
 * The program lives in `minmax_shapes/` because the gate compiles it as its
 * own object against `minmax_shapes.c`; a root-level `.c` would be picked up
 * by `run_regression.py`'s `*.c` glob and fail to link on its own.  The
 * corpus still runs it: `tests/regression/minmax_refused.c` is a complete TU
 * that includes both `minmax_shapes.c` and this driver.  A refused shape must
 * not merely stay scalar, it must stay *correct*, and the cheapest way to keep
 * proving that is to let the ordinary regression corpus run it too.
 */
#include <stdio.h>
#include <limits.h>

unsigned max_u32(const unsigned *a, int n);
short max_i16(const short *a, int n);
short min_i16(const short *a, int n);
float min_f32(const float *a, int n);
int max_guarded(const int *a, int n);
int max_zero_start_dynamic_limit(const int *a, int n);
int max_start1(const int *a, int n);
int max_start1_exit_iv(const int *a, int n, int *out_i);
int max_empty_negative_start(const int *a, int n);
int max_with_volatile_store(const int *a, int n);
int minmax_observer_value(void);
void reset_minmax_observer(int value);
int max_volatile_load(const volatile int *a, int n);
long long sum_volatile_load(const volatile int *a, int n);
int max_with_sum(const int *a, int n, int *sum);
int minmax_pair(const int *a, int n, int *mn);

#define MAXN 512
static int a[MAXN];
static unsigned ua[MAXN];
static short sa[MAXN];
static float fa[MAXN];

static unsigned long long rs = 0x243f6a8885a308d3ULL;
static unsigned long long rnd(void) {
    rs ^= rs << 13; rs ^= rs >> 7; rs ^= rs << 17; return rs;
}
static void mix(unsigned long long v) {
    rs ^= v + 0x9e3779b97f4a7c15ULL + (rs << 6) + (rs >> 2);
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    for (int n = 1; n <= 64; n++) {
        for (int kind = 0; kind < 4; kind++) {
            for (int i = 0; i < n; i++) {
                unsigned long long r = rnd();
                switch (kind) {
                case 0: a[i] = (int)(r & 0xffff) - 32768; break;
                case 1: a[i] = (int)r; break;
                case 2: a[i] = (r & 1) ? INT_MIN : INT_MAX; break;
                default: a[i] = (int)(r % 3) - 1; break;
                }
                ua[i] = (unsigned)a[i];
                sa[i] = (short)(a[i] >> 3);
                fa[i] = (float)a[i];
            }
            if (n > 1) { a[n / 2] = INT_MIN; ua[n / 2] = 0u; sa[n / 2] = SHRT_MIN;
                         fa[n / 2] = -1.0e30f; }
            if (n > 2) { a[n - 1] = INT_MAX; ua[n - 1] = 0xffffffffu;
                         sa[n - 1] = SHRT_MAX; fa[n - 1] = 1.0e30f; }

            int sum = 0, mn = 0, exit_i = 0;
            mix((unsigned long long)(unsigned)max_start1_exit_iv(a, n, &exit_i));
            mix((unsigned long long)(unsigned)exit_i);
            reset_minmax_observer(-777);
            mix((unsigned long long)(unsigned)max_with_volatile_store(a, n));
            mix((unsigned long long)(unsigned)minmax_observer_value());
            mix((unsigned long long)(unsigned)max_volatile_load(a, n));
            mix((unsigned long long)sum_volatile_load(a, n));
            mix((unsigned long long)(unsigned)max_u32(ua, n));
            mix((unsigned long long)(unsigned)(unsigned short)max_i16(sa, n));
            mix((unsigned long long)(unsigned)(unsigned short)min_i16(sa, n));
            mix((unsigned long long)(unsigned)(long long)(min_f32(fa, n) * 7.0f));
            mix((unsigned long long)(unsigned)max_guarded(a, n));
            mix((unsigned long long)(unsigned)max_with_sum(a, n, &sum));
            mix((unsigned long long)(unsigned)sum);
            mix((unsigned long long)(unsigned)minmax_pair(a, n, &mn));
            mix((unsigned long long)(unsigned)mn);
            h = h * 1099511628211ULL ^ rs;
        }
    }
    /* Large lengths: the vector-width boundaries are all below 64. */
    for (int n = 200; n <= MAXN; n += 37) {
        for (int i = 0; i < n; i++) {
            a[i] = (int)rnd();
            ua[i] = (unsigned)a[i];
            sa[i] = (short)(a[i] >> 5);
            fa[i] = (float)(a[i] & 0xffff);
        }
        int sum = 0, mn = 0, exit_i = 0;
        mix((unsigned long long)(unsigned)max_start1_exit_iv(a, n, &exit_i));
        mix((unsigned long long)(unsigned)exit_i);
        reset_minmax_observer(-777);
        mix((unsigned long long)(unsigned)max_with_volatile_store(a, n));
        mix((unsigned long long)(unsigned)minmax_observer_value());
        mix((unsigned long long)(unsigned)max_volatile_load(a, n));
        mix((unsigned long long)sum_volatile_load(a, n));
        mix((unsigned long long)(unsigned)max_u32(ua, n));
        mix((unsigned long long)(unsigned)(unsigned short)max_i16(sa, n));
        mix((unsigned long long)(unsigned)(unsigned short)min_i16(sa, n));
        mix((unsigned long long)(unsigned)(long long)(min_f32(fa, n) * 7.0f));
        mix((unsigned long long)(unsigned)max_guarded(a, n));
        mix((unsigned long long)(unsigned)max_with_sum(a, n, &sum));
        mix((unsigned long long)(unsigned)minmax_pair(a, n, &mn));
        h = h * 1099511628211ULL ^ rs;
    }
    reset_minmax_observer(-777);
    mix((unsigned long long)(unsigned)max_with_volatile_store(a, 0));
    mix((unsigned long long)(unsigned)minmax_observer_value());
    /* A positive-start loop with a dynamic INT_MIN bound is empty. Its
     * c-relative remainder limit must clamp to zero rather than wrap the
     * I32 subtraction `n - c` to a huge positive range. */
    mix((unsigned long long)(unsigned)max_start1(a, INT_MIN));
    /* Zero-start signed bounds: both limits denote empty source loops. The
     * second value makes an I32 `(n / width) * byte_stride` wrap positive if
     * the intermediate is evaluated in the source counter's narrow domain. */
    mix((unsigned long long)(unsigned)max_zero_start_dynamic_limit(
        (const int *)0, -1));
    mix((unsigned long long)(unsigned)max_zero_start_dynamic_limit(
        (const int *)0, -1610612800));
    /* This direct-GEP shape has a nonzero negative start and is also empty.
     * A wrapped absolute byte-bound must not turn it into an out-of-bounds
     * vector load. */
    mix((unsigned long long)(unsigned)max_empty_negative_start(
        (const int *)0, -536870913));
    h = h * 1099511628211ULL ^ rs;
    printf("%016llx\n", h);
    return 0;
}
