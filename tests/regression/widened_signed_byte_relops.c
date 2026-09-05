/*
 * PF-15 signed comparison proof at the C/ABI boundary.  Each helper gets a
 * distinct signed-byte pair so its two integer promotions are single-use;
 * this is the exact form the IR pair rule is allowed to narrow.  Exhausting
 * all 256 x 256 pairs catches the negative-half ordering error that would
 * result from treating sign-extended bytes as unsigned.
 */
#include <stdio.h>

#if defined(__GNUC__)
#define NOINLINE __attribute__((noinline))
#else
#define NOINLINE
#endif

#define REL(name, op) \
    NOINLINE int name(signed char a, signed char b) { return (int)a op (int)b; }

REL(rel_eq, ==)
REL(rel_ne, !=)
REL(rel_lt, <)
REL(rel_le, <=)
REL(rel_gt, >)
REL(rel_ge, >=)

int main(void) {
    unsigned long mismatches = 0;
    unsigned long hash = 2166136261u;

    for (int ai = -128; ai <= 127; ++ai) {
        for (int bi = -128; bi <= 127; ++bi) {
            signed char a = (signed char)ai;
            signed char b = (signed char)bi;
            int got[] = {
                rel_eq(a, b), rel_ne(a, b), rel_lt(a, b),
                rel_le(a, b), rel_gt(a, b), rel_ge(a, b),
            };
            int want[] = {
                ai == bi, ai != bi, ai < bi,
                ai <= bi, ai > bi, ai >= bi,
            };
            for (int k = 0; k != 6; ++k) {
                mismatches += got[k] != want[k];
                hash = (hash ^ (unsigned)got[k]) * 16777619u;
            }
        }
    }

    if (mismatches) {
        printf("widened-signed-byte-relops FAIL %lu\n", mismatches);
        return 1;
    }
    printf("widened-signed-byte-relops OK %lu\n", hash);
    return 0;
}
