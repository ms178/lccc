/* i686 fused Cmp->Select with Ptr compares (emit_fused_cmp_select_impl).
 *
 * A pointer is a full 32-bit unsigned integer on i686, so the fused path
 * shares `cmpl` with I32/U32 and must accept Ptr compares.  It used to
 * assert, which turned the canonical libcall shape
 *
 *     p = memchr(s, c, n);
 *     return p ? p : NULL;          // Cmp Ptr, 0 -> Select Ptr
 *
 * (produced by the mid-end's boolean/select merge) into an internal error.
 * The IR-level memchr idiom makes that shape common, but it was always
 * reachable from plain C: `a == b ? a : b` on pointers.
 *
 * Shapes: rr (two pointer args), ir (constant lhs, null), ri (constant rhs,
 * null), mr (pointer loaded from memory -> load->mem-compare fold), plus the
 * relational predicates restricted to one array so the C program is defined.
 * Every result is folded into an index/hash; stdout is compared with GCC.
 */
#include <stdio.h>

static int cells[8];
static const int *volatile vp; /* read back so the load is not constant-folded */

__attribute__((noinline)) static const int *p_eq(const int *a, const int *b) {
    return a == b ? a : b;
}
__attribute__((noinline)) static const int *p_ne(const int *a, const int *b) {
    return a != b ? a : b;
}
__attribute__((noinline)) static const int *p_lt(const int *a, const int *b) {
    return a < b ? a : b;
}
__attribute__((noinline)) static const int *p_ge(const int *a, const int *b) {
    return a >= b ? a : b;
}
__attribute__((noinline)) static int i_eq(const int *a) {
    return a == 0 ? 11 : -7;
}
__attribute__((noinline)) static long long i_ne(const int *a) {
    return a != 0 ? 0x100000003LL : -5LL;
}
__attribute__((noinline)) static const int *l_eq(const int *a) {
    return 0 == a ? a : cells;
}
__attribute__((noinline)) static int n_ne(void) {
    const int *p = vp;
    return p != 0 ? 3 : 4;
}
__attribute__((noinline)) static const int *n_eq(void) {
    const int *p = vp;
    return p == 0 ? p : cells;
}

static unsigned long long idx(const int *p) {
    return p ? (unsigned long long)(p - cells) + 1 : 0;
}

int main(void) {
    for (int i = 0; i < 8; i++) cells[i] = i * 3;
    unsigned long long h = 1469598103934665603ULL;
    for (int pass = 0; pass < 2; pass++) {
        for (int i = 0; i < 8; i++) {
            for (int j = 0; j < 8; j++) {
                const int *a = &cells[i], *b = &cells[j];
                h = (h ^ idx(p_eq(a, b))) * 1099511628211ULL;
                h = (h ^ idx(p_ne(a, b))) * 1099511628211ULL;
                h = (h ^ idx(p_lt(a, b))) * 1099511628211ULL;
                h = (h ^ idx(p_ge(a, b))) * 1099511628211ULL;
            }
            const int *a = &cells[i];
            h = (h ^ (unsigned)i_eq(a)) * 1099511628211ULL;
            h = (h ^ (unsigned long long)i_ne(a)) * 1099511628211ULL;
            h = (h ^ idx(l_eq(a))) * 1099511628211ULL;
            h = (h ^ (unsigned)i_eq(0)) * 1099511628211ULL;
            h = (h ^ (unsigned long long)i_ne(0)) * 1099511628211ULL;
            h = (h ^ idx(l_eq(0))) * 1099511628211ULL;
        }
        /* Memory-sourced pointer: the second pass takes the other branch. */
        printf("%016llx %d %llu\n", h, n_ne(), idx(n_eq()));
        vp = (pass == 0) ? &cells[5] : 0;
    }
    printf("all %016llx\n", h);
    return 0;
}
