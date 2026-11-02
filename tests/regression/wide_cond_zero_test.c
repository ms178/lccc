// Wide (128-bit) condition zero-tests in `?:`, `if`, `while`, `for`,
// `do-while`, `&&`/`||` and loop-mutated conditions.
//
// Zero-ness of a wide value is (lo | hi) == 0: a value with a zero low half
// and a nonzero high half is NOT zero (`(__int128)1 << 64`). The backend's
// condition zero-test examined only one 64-bit (or worse, 4-byte) slice —
// Select slot homes tested `cmpl $0, slot` (4 bytes of a 16-byte slot!),
// Select accumulator homes `testq %rax, %rax` (low half only), and the
// CondBranch fallback `movq slot, %rax; testq %rax, %rax` (low half only).
// Every such test misclassifies the `1 << 64` class and branches/selects
// the wrong arm.
//
// The fix folds the halves with the GCC shape (`movq lo, %r; orq hi, %r`
// — one flag-setting fold, one control transfer) in branch context, uses a
// read-only rcx-form fold in the cmov select context (flags must survive
// into the cmov), sizes the stack-slot compare to the condition's recorded
// IR type (the PR #368 register-width rule, applied to its stack-slot
// sibling), and routes the legacy pushfq paths through the same fold.
//
// While the wide test now READS the high half of the 16-byte slot, the
// second half of the fix (peephole vector-move extents) keeps that half
// actually defined: `eliminate_dead_stores` models packed vector moves as
// 16-byte range accesses so an i128 parameter's high-half home store is
// never elided just because its only reader is a `movdqu` slot copy.
//
// This is the runtime differential; the assembly-shape half lives in
// check_wide_cond_zero_test.sh.
#include <stdio.h>

typedef unsigned __int128 u128;
typedef struct { unsigned long long lo, hi; } pair128;
static inline __int128 mk(pair128 p) {
    return ((__int128)p.hi << 64) | (__int128)p.lo;
}

int sel(__int128 c, int a, int b) { return c ? a : b; }
int selif(__int128 c, int a, int b) { if (c) return a; return b; }
int negsel(__int128 c, int a, int b) { return !c ? a : b; }
int usel(u128 c, int a, int b) { return c ? a : b; }
int swhile(__int128 c, int a, int b) { int n = 0; while (c) { n += a; break; } return n ? n : b; }
int sfor(__int128 c, int a, int b) { int n = 0; for (; c;) { n += a; break; } return n ? n : b; }
int sdow(__int128 c, int a, int b) { int n = a; do { n += a; } while (0); if (!c) n = b; return n; }
int sand(__int128 c, int a, int b) { return (c && a) ? a : b; }
int sor(__int128 c, int a, int b) { return (c || a) ? a : b; }
int notif(__int128 c, int a, int b) { if (!c) return b; return a; }
int chain(__int128 c, int a, int b) { return c ? (c ? a : b) : (b ? b : a); }
// Loop-mutated wide condition: the re-test each iteration must observe the
// shifted value, in the register allocator's real pressure context.
int loopmut(__int128 c, int a, int b) {
    int n = 0;
    for (int i = 0; i < 3; i++) {
        if (c) n += a; else n += b;
        c = c >> 1;
    }
    return n;
}
// Computed (accumulator-homed) wide condition, not a parameter.
int prodcond(__int128 a, __int128 b, int x, int y) { __int128 p = a * b; return p ? x : y; }
// Wide condition produced by a libcall (call-clobbered homes around it).
int divcond(__int128 a, __int128 b, int x, int y) {
    if (b == 0) return y;
    __int128 q = a / b;
    return q ? x : y;
}
// Thread-local wide condition (memory home beyond the frame).
__thread __int128 gv;
int gset(__int128 v) { gv = v; return 0; }
int gcond(int x, int y) { return gv ? x : y; }

static const pair128 V[] = {
    {0ull, 0ull},                                   /* zero                      */
    {1ull, 0ull},                                   /* low bit                   */
    {0xdeadbeefcafebabeull, 0ull},                  /* low half, high zero       */
    {0ull, 1ull},                                   /* THE `1 << 64` class       */
    {0ull, 0x7fffffffffffffffull},                  /* high half only            */
    {1ull, 1ull},                                   /* both halves               */
    {0xffffffffffffffffull, 0x7fffffffffffffffull}, /* low max, high positive    */
    {0x123456789abcdef0ull, 0x0fedcba987654321ull}, /* mixed                     */
    {0x8000000000000000ull, 0x8000000000000000ull}, /* sign bits                 */
    {0xffffffffffffffffull, 0xffffffffffffffffull}, /* all ones = -1             */
};
#define N (int)(sizeof V / sizeof V[0])

int main(void) {
    long r = 0;
    for (int i = 0; i < N; i++) {
        __int128 c = mk(V[i]);
        r = r * 31 + sel(c, 111, -222);
        r = r * 31 + selif(c, 111, -222);
        r = r * 31 + negsel(c, 111, -222);
        r = r * 31 + usel((u128)c, 111, -222);
        r = r * 31 + swhile(c, 111, -222);
        r = r * 31 + sfor(c, 111, -222);
        r = r * 31 + sdow(c, 111, -222);
        r = r * 31 + sand(c, 111, -222);
        r = r * 31 + sor(c, 111, -222);
        r = r * 31 + notif(c, 111, -222);
        r = r * 31 + chain(c, 111, -222);
        r = r * 31 + loopmut(c, 111, -222);
        r = r * 31 + prodcond(c, mk(V[N - 1 - i]), 111, -222);
        if (i > 0) r = r * 31 + divcond(c, mk(V[i - 1]), 111, -222);
        r = r * 31 + gset(c) + gcond(111, -222);
    }
    printf("%ld\n", r);
    return 0;
}
