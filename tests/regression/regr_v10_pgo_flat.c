// regr_v10_pgo_flat.c
//
// v10 red-team regression: a FLAT-PROFILE workload. Two functions
// (`step_even`, `step_odd`) are called an EQUAL number of times from the hot
// loop, so the profile's hot function set is TIED (no dominant hot path).
// Under the old behavior the percentile thresholds collapsed and the PGO
// inliner acted on the "hot" tie, restructuring hot functions and regressing
// them (zlib-ng adler32 -20%). The `has_spread()` dominance gate must detect
// this flat profile and skip profile-driven inlining, so the `-fprofile-use`
// build's hot functions are NOT bloated relative to plain.
//
// This kernel self-checks correctness under both builds; the round-trip script
// additionally verifies the hot functions are not structurally bloated.
#include <stdio.h>

__attribute__((noinline)) static int step_even(int x) {
    int s = 0;
    s += (x * 3 + 1) & 0xffff;
    s -= (x >> 2);
    s ^= (x << 1) & 0xffff;
    s += (x / 7);
    return s;
}

__attribute__((noinline)) static int step_odd(int x) {
    int s = 0;
    s += (x * 5 - 3) & 0xffff;
    s += (x >> 3);
    s ^= (x * 11) & 0xffff;
    s -= (x / 13);
    return s;
}

__attribute__((noinline)) static int cold_helper(int x) { return x * 2; }

int main(int argc, char** argv) {
    (void)argv;
    long a = 0, b = 0;
    /* equal hotness: step_even and step_odd called the same number of times */
    for (int i = 0; i < 2000000; i++) {
        a += step_even(i & 0xffff);
        b += step_odd(i & 0xffff);
    }
    /* cold path: calls cold_helper once */
    long c = cold_helper(5);
    long e_a = 0, e_b = 0;
    for (int i = 0; i < 2000000; i++) {
        e_a += ((((i & 0xffff) * 3 + 1) & 0xffff) - ((i & 0xffff) >> 2)
                ^ ((i & 0xffff) << 1) & 0xffff) + (((i & 0xffff)) / 7);
        e_b += ((((i & 0xffff) * 5 - 3) & 0xffff) + ((i & 0xffff) >> 3)
                ^ ((i & 0xffff) * 11) & 0xffff) - ((i & 0xffff) / 13);
    }
    if (a != e_a || b != e_b) {
        printf("MISMATCH a=%ld e_a=%ld b=%ld e_b=%ld\n", a, e_a, b, e_b);
        return 1;
    }
    printf("ok %ld %ld %ld\n", a, b, c);
    return 0;
}
