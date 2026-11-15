/*
 * Byte-classifier counting loops, in the two spellings real parsers use
 * (reduced from tests/bench/k_namechars.c and k_classify.c, the expat
 * xml_name_continue family):
 *
 *   1. the COUNTING form `if (pred(s[i])) n++;` — the frontend emits a
 *      jump-threaded `||` chain where every link branches STRAIGHT to the
 *      shared increment block, so no diamond/triangle is ever convertible
 *      until if_convert's combine has folded the chain link by link.  The
 *      combine's profitability simulation must therefore play the caller's
 *      fixpoint (further combine rounds on the probe), and
 *      `is_boolean_condition` must recurse through the And/Or trees earlier
 *      rounds produced, or round 2 never fires and the loop stays scalar.
 *
 *   2. the SUMMED-PREDICATE form `c += (unsigned)pred(s[i]);` through a
 *      small 0/1 function — partial if-conversion leaves ONE pure value
 *      triangle whose ~10-instruction arm the loop-resident cmov budget
 *      rightly refuses; `vectorize_byte_count_normalize` flattens it on a
 *      private clone (plus the cascaded `Select(x,1,y) -> x|y` lowering)
 *      and commits only when the packed byte-count emitter accepts.
 *
 * Both must produce EXACTLY gcc's counts over every byte value (0x00-0xFF,
 * signed-char negative bytes included — the range tests are SIGNED compares
 * and 0x80..0xFF must stay excluded), at trip counts covering the empty
 * loop, sub-packed-width lengths, the 16/32-lane boundaries and a large
 * non-multiple-of-32 length that exercises the scalar remainder.
 */
#include <stdio.h>
#include <string.h>

#define N 1031

static char s[N];

static int is_name_char(char ch) {
    return ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
            (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.');
}

/* Counting form: branchy `||` chain feeding n++. */
static unsigned long long count_form(const char *p, int n) {
    unsigned long long count = 0;
    for (int i = 0; i < n; i++) {
        char c = p[i];
        if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
            (c >= '0' && c <= '9') || c == '_' || c == '-' || c == '.')
            count++;
    }
    return count;
}

/* Summed-predicate form: 0/1 function result accumulated. */
static unsigned long long sum_form(const char *p, int n) {
    unsigned long long c = 0;
    for (int i = 0; i < n; i++) {
        c += (unsigned)is_name_char(p[i]);
    }
    return c;
}

/* Oracle through volatile scalars: no vectorizer can touch this. */
static unsigned long long oracle(const char *p, int n) {
    unsigned long long count = 0;
    for (int i = 0; i < n; i++) {
        volatile char c = p[i];
        volatile int hit = 0;
        if (c >= 'a' && c <= 'z') hit = 1;
        if (c >= 'A' && c <= 'Z') hit = 1;
        if (c >= '0' && c <= '9') hit = 1;
        if (c == '_' || c == '-' || c == '.') hit = 1;
        count += (unsigned)hit;
    }
    return count;
}

int main(void) {
    /* Every byte value at every alignment, boundaries included. */
    for (int i = 0; i < N; i++) {
        s[i] = (char)(unsigned char)(i * 7 + 3);
    }
    /* Pin the exact classifier boundaries into the window the short trip
     * counts below scan, so `n < 32` runs still see them. */
    const char edges[] = {'a', 'z', 'A', 'Z', '0', '9', '_',  '-',  '.',
                          '`', '{', '@', '[', '/', ':', '\0', 0x7f, (char)0x80,
                          (char)0xff, ' '};
    memcpy(s, edges, sizeof(edges));

    unsigned long long h = 1469598103934665603ull;
    int fails = 0;
    static const int trips[] = {0,  1,  2,  3,  15, 16,  17,  31,
                                32, 33, 63, 64, 65, 100, 1000, N};
    for (unsigned t = 0; t < sizeof(trips) / sizeof(trips[0]); t++) {
        int n = trips[t];
        unsigned long long want = oracle(s, n);
        unsigned long long a = count_form(s, n);
        unsigned long long b = sum_form(s, n);
        if (a != want || b != want) {
            printf("MISMATCH n=%d want=%llu count_form=%llu sum_form=%llu\n",
                   n, want, a, b);
            fails++;
        }
        h = (h ^ a) * 1099511628211ull;
        h = (h ^ b) * 1099511628211ull;
    }
    printf("fails=%d hash=%llu\n", fails, h);
    return fails != 0;
}
