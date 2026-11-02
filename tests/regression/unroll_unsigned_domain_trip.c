/* Complete-unroll trip counts must be computed in the comparison's value
 * domain and in the IV's own width (loop_unroll.rs red-team audit).
 *
 * Two miscompiles, both reachable once an OUTER constant-trip loop is
 * completely unrolled and the INNER loop's init/limit becomes an affine
 * constant expression of the substituted outer IV:
 *
 *  1. `resolve_const_operand` narrowed every <= 4-byte BinOp result with
 *     `x as i32 as i64`.  The IR stores unsigned constants zero-extended
 *     (`IrConst::I64(0xFFFFFFF9)` for a U32), so the U32 sum
 *     `0xFFFFFFF8 + 1` folded to -7.  `-7 < 4` gave the inner loop
 *     `j = i + 1; j < 4u` a trip count of 11 although it never executes
 *     (0xFFFFFFF9 < 4 is false).  g1: 4294967287 instead of 0.
 *
 *  2. `complete_unroll_trip` evaluated `Ult/Ule/Ugt/Uge` with signed i64
 *     ordering.  A U64 constant >= 2^63 is a negative i64, so
 *     `0xFFFFFFFFFFFFFFF9 < 4` was "true" with a span of 10.  h1:
 *     18446744073709551607 instead of 0.
 *
 * The fix normalises every folded value through the IR's own constant
 * constructor (`IrConst::from_i64(v, ty).to_i64()`), does the closed form
 * in i128 in the compare's signedness domain, and refuses any IV walk that
 * would leave the IV type's range.  Each kernel below prints a value that
 * differs between the wrong and the right trip count; run_regression_suite
 * diffs against GCC.
 */
#include <stdio.h>

/* (1) U32 affine inner init after outer complete unroll; inner never runs. */
__attribute__((noinline)) unsigned g1(void) {
    unsigned acc = 0, i, j;
    for (i = 0xFFFFFFF8u; i < 0xFFFFFFFAu; i++)
        for (j = i + 1; j < 4u; j++) {
            if (j & 1) acc += j; else acc ^= j;
        }
    return acc;
}

/* (1b) descending U32 inner loop that DOES run 9 times per outer iteration. */
__attribute__((noinline)) unsigned g2(void) {
    unsigned acc = 0, i, j;
    for (i = 0xFFFFFFF8u; i < 0xFFFFFFFAu; i++)
        for (j = i + 1; j > 0xFFFFFFF0u; j--) {
            if (j & 1) acc += j; else acc ^= j;
        }
    return acc;
}

/* (1c) the limit is the affine expression. */
__attribute__((noinline)) unsigned g3(void) {
    unsigned acc = 0, i, j;
    for (i = 0xFFFFFFF8u; i < 0xFFFFFFFAu; i++)
        for (j = 2u; j < i + 8u; j++) {
            if (j & 1) acc += j; else acc ^= j;
        }
    return acc;
}

/* (1d) narrowing cast in the init chain: (unsigned char)(i + 10) wraps. */
__attribute__((noinline)) unsigned g4(void) {
    unsigned acc = 0; unsigned i;
    for (i = 250u; i < 252u; i++) {
        unsigned char j;
        for (j = (unsigned char)(i + 10); j < 6; j++) {
            if (j & 1) acc += j; else acc ^= j * 3u;
        }
    }
    return acc;
}

/* (2) U64 >= 2^63 through a nested affine init; inner never runs. */
__attribute__((noinline)) unsigned long h1(void) {
    unsigned long acc = 0, i, j;
    for (i = 0xFFFFFFFFFFFFFFF8ul; i < 0xFFFFFFFFFFFFFFFAul; i++)
        for (j = i + 1; j < 4ul; j++) {
            if (j & 1) acc += j; else acc ^= j;
        }
    return acc;
}

/* (2b) U64 descending across 2^63 boundary; runs 8 times per outer trip. */
__attribute__((noinline)) unsigned long h2(void) {
    unsigned long acc = 0, i, j;
    for (i = 0x8000000000000003ul; i < 0x8000000000000005ul; i++)
        for (j = i + 1; j > 0x7FFFFFFFFFFFFFFCul; j--) {
            if (j & 1) acc += j; else acc ^= j;
        }
    return acc;
}

/* Signed nesting must still be unrolled correctly (positive control). */
__attribute__((noinline)) int s1(void) {
    int acc = 0, i, j;
    for (i = 0; i < 3; i++)
        for (j = i + 1; j < 6; j++) {
            if (j & 1) acc += j; else acc ^= j;
        }
    return acc;
}

/* Signed INT_MIN neighbourhood: sign extension is the right thing here. */
__attribute__((noinline)) long s2(void) {
    long acc = 0; int i;
    for (i = -2147483647 - 1; i < -2147483646; i++) {
        int j;
        for (j = i + 1; j < -2147483643; j++) {
            if (j & 1) acc += j; else acc ^= j;
        }
    }
    return acc;
}

/* `!=` exit with exact stride divisibility is now completely unrolled;
 * a non-dividing stride must stay a loop and terminate via wrap.  Both
 * must print the same as GCC. */
__attribute__((noinline)) unsigned n1(void) {
    unsigned acc = 0, i;
    for (i = 0; i != 12; i += 3) acc = acc * 31 + i;
    return acc;
}
__attribute__((noinline)) unsigned n2(void) {
    unsigned acc = 0; unsigned char i;
    for (i = 0; i != 7; i += 3) acc = acc * 31 + i;   /* wraps twice before hitting 7 */
    return acc;
}

/* Operand order and polarity: `lim > i`, `!(i >= lim)`, `if (...) break`. */
__attribute__((noinline)) int p1(void) { int s = 0, i; for (i = 0; 5 > i; i++) s += i * 7; return s; }
__attribute__((noinline)) int p2(void) { int s = 0, i; for (i = 0; !(i >= 5); i++) s += i * 11; return s; }
__attribute__((noinline)) int p3(void) { int s = 0, i; for (i = 9; ; i -= 2) { if (i < 2) break; s += i * 13; } return s; }
__attribute__((noinline)) unsigned p4(void) { unsigned s = 0, i; for (i = 3u; 0xFFFFFFF9u <= i; i++) s += i; return s; }

int main(void) {
    printf("g1=%u g2=%u g3=%u g4=%u\n", g1(), g2(), g3(), g4());
    printf("h1=%lu h2=%lu\n", h1(), h2());
    printf("s1=%d s2=%ld\n", s1(), s2());
    printf("n1=%u n2=%u\n", n1(), n2());
    printf("p1=%d p2=%d p3=%d p4=%u\n", p1(), p2(), p3(), p4());
    return 0;
}
