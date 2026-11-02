// PGO branchy:
//
// v9 red-team audit regression: exercises the profile-guided edge consumers
// (block layout + branch-probability fallthrough refinement + switch-case
// ordering) across a PGO generate -> train -> use roundtrip with self-checking
// reference values, and verifies the use build is byte-for-byte behaviorally
// identical to the training build.
//
// The function `classify` has a mix of conditional branches, a dense switch,
// and a hot path (mode 3). The training run drives mode 3 90% of the time, so
// the PGO use build should lay the hot path as fallthrough and reorder switch
// cases — while still producing identical results under both layouts.
#include <stdio.h>

__attribute__((noinline)) static int classify(int x, int mode) {
    int r = 0;
    /* conditional-heavy, hot when mode==3 */
    if (mode == 3) {
        if (x > 1000)      r += (x >> 3) + 1;
        else if (x > 100)  r += (x >> 2) - 2;
        else if (x > 10)   r += (x >> 1) * 3;
        else               r += x * 7;
    } else {
        r += x * 2;
    }
    /* dense switch — case ordering / jump-table decisions */
    switch (x & 7) {
    case 0: r += 1000; break;
    case 1: r += 1001; break;
    case 2: r += 1002; break;
    case 3: r += 1003; break;
    case 4: r += 1004; break;
    case 5: r += 1005; break;
    case 6: r += 1006; break;
    default: r += 1007; break;
    }
    return r;
}

__attribute__((noinline)) static int hot_impl(int a) { return a * 5 + 3; }
__attribute__((noinline)) static int cold_impl(int a) { return a - 9; }

int main(int argc, char** argv) {
    (void)argv;
    int (*f)(int) = hot_impl;
    long s = 0;
    for (int i = 0; i < 200000; i++) {
        int x = (i * 2654435761u) & 0xffff;
        /* 90% mode 3 (hot branch path), 10% mode 0 */
        int mode = (i % 10) < 9 ? 3 : 0;
        s += classify(x, mode);
        /* indirect call — promotion site; hot target dominates */
        s += f(i & 255);
    }
    /* cold path: low-trip, exercises the other mode */
    for (int i = 0; i < 100; i++) s += classify(i, 0);

    /* self-check with an independent formulation */
    long e = 0;
    for (int i = 0; i < 200000; i++) {
        int x = (i * 2654435761u) & 0xffff;
        int mode = (i % 10) < 9 ? 3 : 0;
        int r = 0;
        if (mode == 3) {
            if (x > 1000)      r += (x >> 3) + 1;
            else if (x > 100)  r += (x >> 2) - 2;
            else if (x > 10)   r += (x >> 1) * 3;
            else               r += x * 7;
        } else {
            r += x * 2;
        }
        switch (x & 7) {
        case 0: r += 1000; break; case 1: r += 1001; break;
        case 2: r += 1002; break; case 3: r += 1003; break;
        case 4: r += 1004; break; case 5: r += 1005; break;
        case 6: r += 1006; break; default: r += 1007; break;
        }
        e += r;
        e += hot_impl(i & 255);
    }
    for (int i = 0; i < 100; i++) {
        int x = i, r = x * 2;
        switch (x & 7) {
        case 0: r += 1000; break; case 1: r += 1001; break;
        case 2: r += 1002; break; case 3: r += 1003; break;
        case 4: r += 1004; break; case 5: r += 1005; break;
        case 6: r += 1006; break; default: r += 1007; break;
        }
        e += r;
    }
    if (s != e) {
        printf("MISMATCH s=%ld e=%ld\n", s, e);
        return 1;
    }
    printf("ok %ld\n", s);
    return 0;
}
