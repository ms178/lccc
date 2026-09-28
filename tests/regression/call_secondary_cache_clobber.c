/* CC-O0CALL-1: comparisons park &x in the secondary (%rcx) cache.
 * Staging SysV GP argument 4 overwrites rcx; a later aggregate argument
 * must rematerialize &x, not dereference the scalar/pair now in rcx.
 * The original Csmith seed is 20260945. Even with peepholes disabled,
 * the pre-fix -O0 compiler dereferences NULL in the first call below.
 * Exercise scalar, constant, integer pair and both mixed aggregate ABIs.
 */
union U { unsigned f : 30; unsigned long storage; };
struct Pair { unsigned long lo, hi; };
struct IF { unsigned long i; double f; };
struct FI { double f; unsigned long i; };

__attribute__((noinline)) static unsigned long scalar(
    unsigned a, short b, int c, unsigned long d, union U e) {
    return a + b + c + d + e.f;
}
__attribute__((noinline)) static unsigned long wide2(
    unsigned a, unsigned b, unsigned __int128 c, union U e) {
    return a + b + (unsigned long)c + (unsigned long)(c >> 64) + e.f;
}
__attribute__((noinline)) static unsigned long wide3(
    unsigned a, unsigned b, unsigned c, unsigned __int128 d, union U e) {
    return a + b + c + (unsigned long)d + (unsigned long)(d >> 64) + e.f;
}
__attribute__((noinline)) static unsigned long pair2(
    unsigned a, unsigned b, struct Pair c, union U e) {
    return a + b + c.lo + c.hi + e.f;
}
__attribute__((noinline)) static unsigned long pair3(
    unsigned a, unsigned b, unsigned c, struct Pair d, union U e) {
    return a + b + c + d.lo + d.hi + e.f;
}
__attribute__((noinline)) static unsigned long mixed_if(
    unsigned a, unsigned b, unsigned c, struct IF d, union U e) {
    return a + b + c + d.i + (unsigned long)d.f + e.f;
}
__attribute__((noinline)) static unsigned long mixed_fi(
    unsigned a, unsigned b, unsigned c, struct FI d, union U e) {
    return a + b + c + d.i + (unsigned long)d.f + e.f;
}

int main(void) {
    union U x = {123};
    int z = 0;
    int *p = &z;
    struct Pair pair = {7, 11};
    struct IF ifv = {7, 11.0};
    struct FI fiv = {11.0, 7};
    unsigned __int128 wide = ((unsigned __int128)11 << 64) | 7;
    if (scalar(17, (&x == &x), 3, *p, x) != 144) return 1;
    if (scalar(17, (&x == &x), 3, 0, x) != 144) return 2;
    if (wide2((&x == &x), 3, wide, x) != 145) return 3;
    if (wide3((&x == &x), 3, 5, wide, x) != 150) return 4;
    if (pair2((&x == &x), 3, pair, x) != 145) return 5;
    if (pair3((&x == &x), 3, 5, pair, x) != 150) return 6;
    if (mixed_if((&x == &x), 3, 5, ifv, x) != 150) return 7;
    if (mixed_fi((&x == &x), 3, 5, fiv, x) != 150) return 8;
    return 0;
}
