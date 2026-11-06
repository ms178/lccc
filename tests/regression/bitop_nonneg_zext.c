/* Clz/Ctz/Popcount results are provably in [0, bitwidth]: their I32→I64
 * signed widening casts do not need a sign-extend on any backend (x86-64
 * reclassifies to movl via bitop_nonneg_values; AArch64 skips the sxtw
 * because the W-register write already zeroed the upper half; RISC-V skips
 * the sext.w because the count was built from 0 upward in a full register).
 * The plain-integer controls MUST keep a sign-extension where one is
 * needed: an AArch64 `int` argument has unspecified upper bits (AAPCS64), so
 * widen_plain keeps its sxtw; a RISC-V `int` argument already arrives
 * sign-extended to XLEN (RISC-V psABI), so there widen_plain is a bare `ret`
 * (Clang 23.1.5 emits the same) and widen_trunc -- the low word of a full
 * 64-bit value -- is the control that keeps its sext.w. Behavioral pin for
 * the x86-64 host run; check_bitop_nonneg_zext.sh pins the AArch64 / RISC-V
 * codegen shapes.
 */
long widen_clz(unsigned long long x) { return (long)__builtin_clz((unsigned)x); }
long widen_ctz(unsigned long long x) { return (long)__builtin_ctz((unsigned)x); }
long widen_pop(unsigned long long x) { return (long)__builtin_popcount((unsigned)x); }
long widen_plain(int x) { return (long)x; } /* control: keeps the extend */
long widen_trunc(long x) { return (long)(int)x; } /* control on every target */

int main(void) {
    unsigned v = 0x00F00000u | 1u;
    long a = widen_clz(v);
    long b = widen_ctz(v);
    long c = widen_pop(v);
    long d = widen_plain(-1);
    if (a != 8 || b != 0 || c != 5 || d != -1)
        return 1;
    if (widen_clz(0) != 32 || widen_ctz(0) != 32 || widen_pop(0) != 0)
        return 2;
    if (widen_plain(-5) != -5)
        return 3;
    if (widen_trunc(0x180000000L) != -2147483648L || widen_trunc(0x7fffffffL) != 0x7fffffffL)
        return 4;
    return 0;
}
