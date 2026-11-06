/* Self-checking WIDE parameter-home fixture: 48 parameters, mixed widths.
 *
 * 40 `int` + 8 `long long` exceed every supported ABI's argument-register
 * budget (x86-64 SysV 6/6, i686 0, AArch64 8, RV64 8), so most parameters
 * arrive in stack slots and must be homed through the index.  A volatile
 * local shifts the homes off the aligned layout.  The `long long` half also
 * exercises width-dependent alignment on 32-bit i686, where a `long long`
 * parameter occupies two stack words and the prologue must not confuse it
 * with the 4-byte homes.
 *
 * Expected checksum is hardcoded (820 + 36000 == 36820), so a compiler that
 * drops or duplicates a parameter cannot pass by agreeing with itself.
 * Freestanding: no includes, no libc, no global initializers.
 */

long long wide(int p0, int p1, int p2, int p3, int p4, int p5, int p6, int p7,
               int p8, int p9, int p10, int p11, int p12, int p13, int p14, int p15,
               int p16, int p17, int p18, int p19, int p20, int p21, int p22, int p23,
               int p24, int p25, int p26, int p27, int p28, int p29, int p30, int p31,
               int p32, int p33, int p34, int p35, int p36, int p37, int p38, int p39,
               long long q0, long long q1, long long q2, long long q3,
               long long q4, long long q5, long long q6, long long q7)
{
    volatile long long local = q0;
    long long s = 0;
    s += p0; s += p1; s += p2; s += p3; s += p4; s += p5; s += p6; s += p7;
    s += p8; s += p9; s += p10; s += p11; s += p12; s += p13; s += p14; s += p15;
    s += p16; s += p17; s += p18; s += p19; s += p20; s += p21; s += p22; s += p23;
    s += p24; s += p25; s += p26; s += p27; s += p28; s += p29; s += p30; s += p31;
    s += p32; s += p33; s += p34; s += p35; s += p36; s += p37; s += p38; s += p39;
    s += q0; s += q1; s += q2; s += q3; s += q4; s += q5; s += q6; s += q7;
    if (local != q0)
        return -1;
    return s;
}

int main(void)
{
    long long r = wide(
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
        33, 34, 35, 36, 37, 38, 39, 40,
        1000LL, 2000LL, 3000LL, 4000LL, 5000LL, 6000LL, 7000LL, 8000LL);
    if (r != 36820LL)
        return 1;
    return 0;
}
