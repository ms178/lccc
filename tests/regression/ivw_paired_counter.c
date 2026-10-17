/* Paired-counter induction proof (iv_widen sibling mirror), executed.
 *
 * A testless zero-seeded `i += 1` is bounded by a strict guard on a
 * same-type sibling `j -= 1`.  Every runtime bound below is passed through a
 * noinline boundary, so the proof sees symbolic seeds and limits.  The
 * narrow-type edges (INT_MIN and INT_MAX neighbourhoods) are exercised
 * explicitly: a sibling mirror that let the narrow counter wrap, or that
 * widened a counter whose final latch value escapes the loop, would change
 * the checksum.  The shapes that must be refused (counter observed after the
 * loop, non-strict guard) are included so the refusal path is also executed.
 *
 * Oracle: gcc -O0 output.  Build with -fwrapv (the signed-wrap policy under
 * which the mirror proof is claimed).  Exit status 0 on the expected checksum.
 */
#include <limits.h>
#include <stdio.h>

/* Every driver call runs at most 700 body passes, so buf[i] stays in range. */
static unsigned char buf[4096];

/* Counter i is not observed after the loop: the mirror proof may apply. */
__attribute__((noinline)) static unsigned long long pc_quiet(int lo, int hi) {
    unsigned long long acc = 0;
    int i = 0;
    int j = hi;
    while (j > lo) {
        acc = acc * 31ULL + buf[i];
        i++;
        j--;
    }
    return acc;
}

/* Counter i is observed after the loop: the final latch value escapes, so the
 * wrap-tolerant proof must refuse and the narrow counter must stay narrow. */
__attribute__((noinline)) static unsigned long long pc_escape(int lo, int hi) {
    unsigned long long acc = 0;
    int i = 0;
    int j = hi;
    while (j > lo) {
        acc = acc * 31ULL + buf[i];
        i++;
        j--;
    }
    return acc ^ (unsigned long long)(unsigned)i;
}

/* Non-strict guard (j >= lo): outside the admitted strict-header subset. */
__attribute__((noinline)) static unsigned long long pc_nonstrict(int lo, int hi) {
    unsigned long long acc = 0;
    int i = 0;
    int j = hi;
    while (j >= lo) {
        acc = acc * 131ULL + buf[i];
        i++;
        j--;
    }
    return acc;
}

/* Store through the counter as a 64-bit address: a mirrored counter feeding a
 * store must keep the same memory image. */
__attribute__((noinline)) static unsigned long long pc_store(int lo, int hi) {
    unsigned long long acc = 0;
    int i = 0;
    int j = hi;
    while (j > lo) {
        buf[i] = (unsigned char)(buf[i] + j);
        acc += buf[i];
        i++;
        j--;
    }
    return acc;
}

int main(void) {
    static const int lows[] = {INT_MIN, INT_MIN + 1, -1000, -3, -1, 0, 1, 5, INT_MAX - 300};
    unsigned long long sum = 0;
    unsigned seed = 12345u;
    for (unsigned k = 0; k < sizeof buf; ++k) {
        seed = seed * 1103515245u + 12345u;
        buf[k] = (unsigned char)(seed >> 16);
    }
    for (unsigned a = 0; a < sizeof lows / sizeof lows[0]; ++a) {
        int lo = lows[a];
        /* Trip counts 0..6 at the lower edge, and a moderate body count. */
        for (int d = 0; d <= 6; ++d) {
            int hi = (int)((long long)lo + d);
            if ((long long)hi < INT_MIN || (long long)hi > INT_MAX) continue;
            sum = sum * 1000003ULL + pc_quiet(lo, hi);
            sum = sum * 1000003ULL + pc_escape(lo, hi);
            /* j >= INT_MIN never fails, so the non-strict shape is only defined above INT_MIN. */
            if (lo > INT_MIN) sum = sum * 1000003ULL + pc_nonstrict(lo, hi);
            sum = sum * 1000003ULL + pc_store(lo, hi);
        }
        for (int d = 7; d <= 700; d += 97) {
            long long hi64 = (long long)lo + d;
            if (hi64 > INT_MAX) continue;
            int hi = (int)hi64;
            sum = sum * 1000003ULL + pc_quiet(lo, hi);
            sum = sum * 1000003ULL + pc_store(lo, hi);
        }
    }
    /* Descending sibling strictly above the lower bound, near INT_MAX. */
    sum = sum * 1000003ULL + pc_quiet(INT_MAX - 5, INT_MAX);
    sum = sum * 1000003ULL + pc_quiet(INT_MAX - 1, INT_MAX);
    sum = sum * 1000003ULL + pc_quiet(INT_MAX, INT_MAX);
    printf("ivw_paired_counter %llu\n", sum);
    return 0;
}
