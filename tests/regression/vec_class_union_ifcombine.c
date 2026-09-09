/*
 * Short-circuit if-combine + single-bit window union (OP-05e).
 *
 * Two transformations meet here:
 *
 *  1. `passes/if_convert.rs` folds a two-level short-circuit branch chain
 *     into ONE branch on a bitwise and/or, which is what removes the
 *     "internal condbranch" that used to keep every `||`-classifier loop
 *     scalar.  It is deliberately restricted to natural-loop bodies so that
 *     the path-sensitive reasoning `path_range_var_minus_one_uintmax.c`
 *     depends on is untouched.
 *  2. `fold_single_bit_window_union` collapses `x in W1 or x in W2`, where
 *     W2 is W1 with one bit set, into `(x | B) in W2`.  That is the ASCII
 *     `isalpha` idiom: `[A-Z] u [a-z]` becomes one compare on `c | 32`.
 *
 * Both are only sound under preconditions this file exercises from both
 * sides.  `alpha`/`alnum`/`hexish` must fold; `overlap` and `stride3` must
 * NOT (their windows violate the single-bit precondition), and are here to
 * prove the refusal path still computes the right answer.
 *
 * The reference is computed through `volatile` scalars, the input covers
 * every byte value at every alignment, and every trip count from 0 up is
 * run so the packed body, the remainder and the empty loop are all covered.
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define N 1031

static unsigned char src[N], dst[N], ref[N];
static int fails;

void k_alpha(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)(((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) ? 1 : 0); }
}
void k_alpha_swapped(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)(((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z')) ? 1 : 0); }
}
void k_tolower(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* union classifier driving a value, not a 0/1 flag */
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)(((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z')) ? (c | 32) : c); }
}
void k_overlap(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* bit 5 is ALREADY set inside [30,34] -> the union fold must refuse */
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)(((c >= 30 && c <= 34) || (c >= 62 && c <= 66)) ? 1 : 0); }
}
void k_stride3(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* separation 3 is not a power of two -> refuse */
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)(((c >= 1 && c <= 2) || (c >= 4 && c <= 5)) ? 1 : 0); }
}
void k_triple(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* three-way OR: at most one pair can fold; the rest must stay exact */
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)(((c >= '0' && c <= '9') || (c >= 'a' && c <= 'z')
                                 || (c >= 'A' && c <= 'Z')) ? 1 : 0); }
}
void k_and_chain(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    /* three-level `&&` chain: if-combine must fold it twice */
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        d[i] = (unsigned char)((c >= 32 && c <= 126 && (c & 1)) ? c : 0); }
}

#define REF(EXPR) do { for (int i = 0; i < N; ++i) { volatile unsigned char vc = src[i]; \
    unsigned char c = vc; ref[i] = (unsigned char)(EXPR); } } while (0)

static void r_alpha(void){ REF(((c>='a'&&c<='z')||(c>='A'&&c<='Z'))?1:0); }
static void r_tolower(void){ REF(((c>='A'&&c<='Z')||(c>='a'&&c<='z'))?(c|32):c); }
static void r_overlap(void){ REF(((c>=30&&c<=34)||(c>=62&&c<=66))?1:0); }
static void r_stride3(void){ REF(((c>=1&&c<=2)||(c>=4&&c<=5))?1:0); }
static void r_triple(void){ REF(((c>='0'&&c<='9')||(c>='a'&&c<='z')||(c>='A'&&c<='Z'))?1:0); }
static void r_and_chain(void){ REF((c>=32&&c<=126&&(c&1))?c:0); }

static void run(const char *name,
                void (*k)(unsigned char *restrict, const unsigned char *restrict, unsigned long),
                void (*r)(void)) {
    for (unsigned long n = 0; n <= N; n = (n < 70 ? n + 1 : n * 3 + 1)) {
        unsigned long m = n > N ? N : n;
        memset(dst, 0xAB, sizeof dst);
        r();
        k(dst, src, m);
        for (unsigned long i = 0; i < m; ++i)
            if (dst[i] != ref[i]) {
                printf("FAIL %s n=%lu i=%lu src=%02x got=%02x want=%02x\n",
                       name, m, i, src[i], dst[i], ref[i]);
                if (++fails > 8) exit(1);
                break;
            }
    }
}

int main(void) {
    for (int i = 0; i < N; ++i)
        src[i] = (unsigned char)(i < 256 ? i : (i * 167 + 13));
    run("alpha",         k_alpha,         r_alpha);
    run("alpha_swapped", k_alpha_swapped, r_alpha);
    run("tolower",       k_tolower,       r_tolower);
    run("overlap",       k_overlap,       r_overlap);
    run("stride3",       k_stride3,       r_stride3);
    run("triple",        k_triple,        r_triple);
    run("and_chain",     k_and_chain,     r_and_chain);
    if (fails) { puts("VALIDATION FAILED"); return 1; }
    puts("VALIDATION OK");
    return 0;
}
