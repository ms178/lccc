/* Sibling calls, branch-free selects, call-argument staging and pointer
 * loop exits on AArch64 and RISC-V (check_switch_cross_execute.py).
 *
 * Every check compares one result with the value GCC 16.2 computes for the
 * same C on x86-64 (`-DGEN` prints the table below).  The C has no undefined
 * behaviour (inputs keep every signed operation in range) and relies only on
 * LP64 and on the modulo conversion to narrow signed types that GCC, Clang
 * and LCCC all define, so the values are the same on every target.  main
 * returns the 1-based index of the first mismatch (254 from there on; the
 * global `n__` holds the exact index), 255 for a check count mismatch.
 *
 * What the shapes pin:
 *   - sibling calls (`b`/`tail`, `br x16`/`jr t2`): argument rotations whose
 *     staging is a register cycle, narrow arguments the callee extends,
 *     6 and 8 register arguments, indirect targets, branch arms (`ret_dup`),
 *     void tails, FP tails, and the frameless `every_call_is_sibling` frame;
 *   - branch-free selects on RV64 (`snez`/`seqz` + `slli`/`neg`/`addi -1;
 *     andi` + `add[w]`/`or`/`xor`/`addi`) at the imm12 and shift edges, with
 *     the condition both a compare result and a plain integer, and a select
 *     whose shared operand IS the condition (`c ? c + 4 : c`);
 *   - call staging hazards: the k-th argument read after the earlier
 *     argument registers are written (AArch64 x0/x9.., RISC-V a0..a7);
 *   - pointer exit tests (LFTR) for trip counts -1, 0, 1 and 7, unsigned,
 *     inclusive, stride-2 and descending counters;
 *   - a direct nested-function call passing a half-precision FP struct on
 *     RV64 (the NaN-boxing scratch must not be t2, the static chain).
 */
#ifdef GEN
#include <stdio.h>
#endif

#define NI __attribute__((noinline))

typedef long L;
typedef unsigned long UL;

/* ------------------------------------------------------------ callees */
NI int take_two(int a, int b) { return a * 5 - b; }
NI int take_s8(signed char c) { return c * 3; }
NI unsigned take_u16(unsigned short c) { return c + 7u; }
NI L take_l(L a) { return a ^ 0x55; }
NI void sink_v(int *p, int v) { *p += v; }
NI L six(L a, L b, L c, L d, L e, L f) { return a - 2 * b + 3 * c - 4 * d + 5 * e - 6 * f; }
NI L eight(L a, L b, L c, L d, L e, L f, L g, L h)
{
    return a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h;
}
NI double fmix(double x, int k, double y) { return x * k - y; }
NI int mix3(int a, unsigned char b, short c) { return a * 7 + b - c; }
typedef int (*fp2)(int, int);
typedef L (*fp6)(L, L, L, L, L, L);

/* ------------------------------------------------------ sibling calls */
NI int tail1(int x) { return take_s8((signed char)(x + 1)); }
NI int tail_swap(int a, int b) { return take_two(b, a); }
NI int tail_add(int a, int b) { return take_two(a + b, 7); }
NI L tail_l(L a) { return take_l(a + 9); }
NI void tail_void(int *p) { sink_v(p, 3); }
NI L tail_rot6(L a, L b, L c, L d, L e, L f) { return six(f, a, b, c, d, e); }
NI L tail_rev8(L a, L b, L c, L d, L e, L f, L g, L h) { return eight(h, g, f, e, d, c, b, a); }
NI int tail_ind(fp2 g, int a, int b) { return g(b, a); }
NI L tail_ind6(fp6 g, L a, L b, L c, L d, L e) { return g(e, d, c, b, a, 11); }
NI int arms(int c, int x) { return c ? take_two(x, c) : take_s8((signed char)x); }
NI void arms_void(int c, int *p)
{
    if (c & 1)
        sink_v(p, c);
    else
        sink_v(p, -c);
}
NI double tail_fp(double x, int k) { return fmix(x, k + 1, x); }
NI int cond_tail(int a, int b)
{
    if (a > b)
        return take_two(a, b);
    return b - a;
}
NI int keep_b(int a, int b) { int t = take_two(a, 1); return t + b; }
NI int chain2(int a, int b) { int t = take_two(a, b); return take_two(t, a); }
NI unsigned pass_u16(unsigned x) { return take_u16((unsigned short)x) * 2; }
NI int tail_narrow3(int a, int b, int c) { return mix3(c, (unsigned char)a, (short)b); }

/* ------------------------------------------------- staging hazards */
NI L stage5(L a, L b, L c, L d, L e) { return six(e, d, c, b, a, a + e) + 1; }
NI L stage8(L a, L b, L c, L d, L e, L f, L g, L h) { return eight(b, a, d, c, f, e, h, g) + 1; }
NI L stage_mixed(L a, L b, L c, L d) { return six(d, c, b, a, d * 2, c - a) ^ eight(a, b, c, d, a, b, c, d); }

/* -------------------------------------------------- branch-free selects */
NI int sel_pow2(int c, int x) { return c ? x + 8 : x; }
NI int sel_add1(int c, int x) { return c ? x + 1 : x; }
NI int sel_add100(int c, int x) { return c ? x + 100 : x; }
NI int sel_add2047(int c, int x) { return c ? x + 2047 : x; }
NI int sel_add2048(int c, int x) { return c ? x + 2048 : x; }
NI int sel_sub3(int c, int x) { return c ? x - 3 : x; }
NI int sel_sub2048(int c, int x) { return c ? x - 2048 : x; }
NI int sel_inv5(int c, int x) { return c ? x : x + 5; }
NI L sel_add8_l(L c, L x) { return c ? x + 8 : x; }
NI L sel_shl62(L c, L x) { return c ? x + (1L << 62) : x; }
NI int sel_shl30(int c, int x) { return c ? x + (1 << 30) : x; }
NI int sel_or16(int c, int x) { return c ? x | 16 : x; }
NI int sel_orm16(int c, int x) { return c ? x | -16 : x; }
NI int sel_xor9(int a, int b, int x) { return a == b ? x : x ^ 9; }
NI L sel_xor_l(L a, L b, L x) { return a < b ? x ^ 0x7ff : x; }
NI int sel_m1(int c) { return c ? -1 : 0; }
NI int sel_8(int c) { return c ? 8 : 0; }
NI int sel_k(int c) { return c ? 0 : 1000; }
NI int sel_45(int c) { return c ? 4 : 5; }
NI int sel_ab(int c) { return c ? 1029 : 5; }
NI int sel_big(int c) { return c ? 0x7fffffff : -1; }
NI L sel_cond_is_x(L c) { return c ? c + 4 : c; }
NI int sel_cmp_slt(int a, int b, int x) { return a < b ? x + 4 : x; }
NI UL sel_u64(UL c, UL x) { return c ? x + 16 : x; }
NI unsigned sel_u32(unsigned c, unsigned x) { return c ? x + 4u : x; }
NI L sel_upper(L c, L x) { return c ? x - 1 : x; }

/* ------------------------------------------------ pointer loop exits */
NI L sum_n(const int *p, L n) { L s = 0; for (L i = 0; i < n; i++) s += p[i]; return s; }
NI L sum_u(const int *p, unsigned n) { L s = 0; for (unsigned i = 0; i < n; i++) s += p[i]; return s; }
NI L sum_incl(const int *p, int n) { L s = 0; for (int i = 0; i <= n; i++) s += p[i]; return s; }
NI L sum_step2(const int *p, int n) { L s = 0; for (int i = 0; i < n; i += 2) s += p[i]; return s; }
NI L sum_down(const int *p, int n) { L s = 0; for (int i = n; i > 0; i--) s += p[i - 1] * i; return s; }
NI L sum_ne(const short *p, L n) { L s = 0; for (L i = 0; i != n; i++) s += p[i]; return s; }

/* ------------------------------ nested function with a half FP struct */
#if !defined(__clang__)
struct hf { _Float16 h; float f; };
NI int nested_half(int k)
{
    int inner(struct hf v) { return (int)(v.h * 4) + (int)v.f + k; }
    struct hf v = { (_Float16)1.75f, 2.5f + k };
    return inner(v);
}
#endif

/* Magnitudes keep every signed operation in range (no overflow). */
volatile int vi[16] = { 0, 1, -1, 2, 7, -8, 100, 1 << 20, -(1 << 20), 3, 5, 9, 42, -42, 13, 65535 };
/* 1 << 32: a nonzero condition with a zero low word. */
volatile L vl[8] = { 0, 1, -1, 1L << 32, -(1L << 40), 123456789L, 9, -9 };
static int data[16] = { 3, -1, 4, 1, -5, 9, 2, -6, 5, 3, -5, 8, 9, -7, 9, 3 };
static short sdata[8] = { 100, -200, 300, -400, 500, -600, 700, -800 };

#ifdef GEN
#define CHECK(e) printf("    %lldLL,\n", (long long)(e))
#else
/* Generated: gcc-16 -O2 -DGEN sibcall_select_staging_cross.c && ./a.out
 * (identical at -O0, and with Clang 23.1.5 up to the nested-function
 * checks it cannot compile). */
static const long long expected[] = {
    3LL, -40LL, -47LL, -24LL, 8LL, -9LL, 40LL, 14LL, -48LL, -40LL, -8LL, -8LL, -8LL, -8LL, -8LL,
    -8LL, -8LL, -3LL, -8LL, -8LL, -8LL, 9LL, 0LL, 0LL, 0LL, 1000LL, 5LL, 5LL, -1LL, 0LL,
    4294967288LL, 11LL, 0LL, 6LL, 499LL, 498LL, 499LL, 99LL, 104LL, -476LL, 16LL, 608LL, 499LL,
    108LL, 101LL, 100LL, 2147LL, 2148LL, 97LL, -1948LL, 100LL, 1073741924LL, 116LL, -12LL, 8LL,
    1LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 5LL, 104LL, -96LL, 16LL, 0LL, 5242881LL,
    5242868LL, 5242877LL, 1048577LL, 1048570LL, -5242904LL, 131084LL, -7339784LL, 5242881LL,
    1048584LL, 1048577LL, 1048676LL, 1050623LL, 1050624LL, 1048573LL, 1046528LL, 1048576LL,
    1074790400LL, 1048592LL, -16LL, 11LL, 2LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 6LL,
    1048580LL, -1048574LL, 0LL, 9LL, -5242882LL, -5242877LL, -5242882LL, 1048586LL, -1048567LL,
    5242928LL, 18LL, -7340016LL, -5242882LL, -1048576LL, -1048575LL, -1048476LL, -1046529LL,
    -1046528LL, -1048579LL, -1050624LL, -1048576LL, 1072693248LL, -1048560LL, -16LL, 10LL, 3LL,
    -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 3LL, 4293918724LL, 1048581LL, 0LL, 24LL, 8LL,
    43LL, 12LL, 32LL, 37LL, 153LL, 28LL, 32LL, 8LL, 11LL, 4LL, 103LL, 2050LL, 2051LL, 0LL,
    -2045LL, 3LL, 1073741827LL, 19LL, -13LL, 13LL, 4LL, -1LL, 8LL, 0LL, 4LL, 1029LL,
    2147483647LL, 4LL, 7LL, 13LL, 84LL, -21LL, 33LL, -22LL, 15LL, 13LL, -36LL, -217LL, 131070LL,
    222LL, 33LL, 5LL, 6LL, 5LL, 2052LL, 2053LL, 2LL, -2043LL, 5LL, 1073741829LL, 21LL, -11LL,
    12LL, 5LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 9LL, 9LL, 0LL, -160LL, 303LL, -55LL,
    538LL, 27LL, 491LL, 508LL, 2355LL, 214LL, 854LL, -55LL, 9LL, 10LL, 9LL, 2056LL, 2057LL, 6LL,
    -2039LL, 9LL, 1073741833LL, 25LL, -7LL, 15LL, 6LL, -1LL, 8LL, 0LL, 4LL, 1029LL,
    2147483647LL, 6LL, 13LL, 112LL, 400LL, 3LL, -1048366LL, 5243083LL, 126LL, 5242838LL,
    5242921LL, 25165614LL, 14LL, 7340284LL, -1048366LL, 42LL, 43LL, 42LL, 2089LL, 2090LL, 39LL,
    -2006LL, 42LL, 1073741866LL, 58LL, -6LL, 14LL, 7LL, -1LL, 8LL, 0LL, 4LL, 1029LL,
    2147483647LL, 7LL, 46LL, 1048537LL, 8388608LL, 3LL, 1048366LL, -5243097LL, -126LL,
    1048534LL, -5242923LL, -25165614LL, 14LL, 7339780LL, 1048366LL, -42LL, -41LL, -42LL, 2005LL,
    2006LL, -45LL, -2090LL, -42LL, 1073741782LL, -42LL, -10LL, 1LL, 8LL, -1LL, 8LL, 0LL, 4LL,
    1029LL, 2147483647LL, 12LL, 4294967258LL, -1048531LL, -25165824LL, 12LL, 62LL, 73LL, 62LL,
    10LL, 27LL, 7LL, 20LL, 88LL, 62LL, 21LL, 14LL, 113LL, 2060LL, 2061LL, 10LL, -2035LL, 13LL,
    1073741837LL, 29LL, -3LL, 0LL, 9LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 13LL, 17LL,
    19LL, 60LL, 18LL, 327670LL, 327693LL, 327674LL, 65530LL, 65559LL, -327555LL, 24LL, 458716LL,
    327670LL, 65543LL, 65536LL, 65535LL, 67582LL, 67583LL, 65532LL, 63487LL, 65535LL,
    1073807359LL, 65535LL, -1LL, 3LL, 10LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 14LL,
    65539LL, 65543LL, 140LL, 30LL, -9LL, 38LL, -1LL, 45LL, 44LL, 216LL, 32LL, 72LL, -9LL, 8LL,
    1LL, 0LL, 2047LL, 2048LL, -3LL, -2048LL, 0LL, 1073741824LL, 16LL, -16LL, 2LL, 11LL, -1LL,
    8LL, 0LL, 4LL, 1029LL, 2147483647LL, 11LL, 4LL, 12LL, 0LL, 129LL, -37LL, 208LL, 3LL, 209LL,
    210LL, 1003LL, 98LL, 342LL, -37LL, 1LL, 2LL, 101LL, 2048LL, 2049LL, -2LL, -2047LL, 1LL,
    1073741825LL, 17LL, -15LL, 5LL, 12LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 12LL, 5LL,
    46LL, 168LL, -123LL, 37LL, -222LL, -7LL, 41LL, -212LL, -1003LL, 131002LL, 502LL, 37LL, -1LL,
    0LL, 99LL, 2046LL, 2047LL, -4LL, -2049LL, -1LL, 1073741823LL, -1LL, -1LL, 4LL, 13LL, -1LL,
    8LL, 0LL, 4LL, 1029LL, 2147483647LL, 17LL, 3LL, -40LL, -1176LL, 42LL, -3LL, 68LL, 9LL, 63LL,
    66LL, 302LL, 40LL, 116LL, -3LL, 10LL, 3LL, 2LL, 2049LL, 2050LL, -1LL, -2046LL, 2LL,
    1073741826LL, 18LL, -14LL, 7LL, 14LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 14LL, 6LL,
    14LL, 104LL, 0LL, -65500LL, 327703LL, 32LL, 327668LL, 327681LL, 1572805LL, 131084LL,
    458944LL, -65500LL, 15LL, 8LL, 107LL, 2054LL, 2055LL, 4LL, -2041LL, 7LL, 1073741831LL, 23LL,
    -9LL, 6LL, 15LL, -1LL, 8LL, 0LL, 4LL, 1029LL, 2147483647LL, 15LL, 11LL, 65545LL, 1834980LL,
    92LL, -13008358666LL, 39395446464LL, -16809498884LL, -16192214871LL, 35717763146LL,
    45310514570LL, 4294967296LL, 4294967296LL, 123456234LL, 0LL, 4294967296LL, 123456789LL,
    95LL, 3298534883333LL, -9895604649854LL, 4398046511069LL, 4398046511169LL, -8796093022003LL,
    5497558138793LL, -1099511627768LL, 4611684918915760128LL, 9LL, 5LL, -1099511627760LL, 8LL,
    93LL, -370370350LL, 617283947LL, -493827255LL, -493827225LL, 987654320LL, 751636475LL,
    123456797LL, 4611686018550844693LL, -2040LL, 3LL, 123456805LL, -10LL, 4294967388LL,
    12884901872LL, 42949673107LL, 21474836377LL, -25769803817LL, 8589934778LL, 17179869264LL,
    17LL, 4611686018427387913LL, 0LL, 4294967300LL, 25LL, -1LL, -1099511627684LL,
    -3298534883291LL, -6597069766665LL, -5497558138908LL, 6597069766693LL, -2199023255494LL,
    8796093022203LL, -1LL, 4611686018427387895LL, 2046LL, -1099511627772LL, 7LL, 0LL,
    123456843LL, 370370379LL, 1234567950LL, 617283875LL, -740740751LL, 246913691LL, 579949785LL,
    8LL, 4611686018427387904LL, -1LL, 123456793LL, 16LL, -2LL, 71LL, -4294967261LL,
    25769803937LL, 12884901862LL, 34359738299LL, 47244640411LL, -60129541935LL, 9LL,
    4611686018427387905LL, 4294967296LL, 13LL, 17LL, 4294967295LL, 85LL, 1099511627763LL,
    -6597069766653LL, -3298534883436LL, -8796093022163LL, -12094627905428LL, -2199023255506LL,
    7LL, 4611686018427387903LL, -1099511625729LL, -5LL, 15LL, -1099511627777LL, 0LL, 0LL, 0LL,
    0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 3LL, 3LL, 3LL, 3LL, 3LL, 100LL, 13LL, 13LL,
    13LL, 4LL, 60LL, 400LL, 32LL, 32LL, 32LL, 22LL, 330LL, -400LL, 5LL, 7LL, 9LL, 11LL, 13LL,
};
/* The index of the next check; a debugger reads the failing one here. */
volatile int n__;
#define CHECK(e)                                     \
    do {                                             \
        if ((long long)(e) != expected[n__])         \
            return n__ < 254 ? n__ + 1 : 254;        \
        n__ = n__ + 1;                               \
    } while (0)
#endif

int main(void)
{
    for (int i = 0; i < 16; i++) {
        int x = vi[i], y = vi[(i + 5) & 15];
        CHECK(tail1(x));
        CHECK(tail_swap(x, y));
        CHECK(tail_add(x, y));
        CHECK(arms(x & 3, y));
        CHECK(cond_tail(x, y));
        CHECK(keep_b(x, y));
        CHECK(chain2(x, y));
        CHECK(pass_u16((unsigned)x));
        CHECK(tail_narrow3(x, y, x ^ y));
        CHECK(tail_ind(take_two, x, y));
        CHECK(sel_pow2(x & 1, y));
        CHECK(sel_add1(x, y));
        CHECK(sel_add100(x & 2, y));
        CHECK(sel_add2047(x, y));
        CHECK(sel_add2048(x, y));
        CHECK(sel_sub3(x, y));
        CHECK(sel_sub2048(x, y));
        CHECK(sel_inv5(x, y));
        CHECK(sel_shl30(x, y));
        CHECK(sel_or16(x, y));
        CHECK(sel_orm16(x, y));
        CHECK(sel_xor9(x, y, i));
        CHECK(sel_xor9(x, x, i));
        CHECK(sel_m1(x));
        CHECK(sel_8(x));
        CHECK(sel_k(x));
        CHECK(sel_45(x));
        CHECK(sel_ab(x));
        CHECK(sel_big(x));
        CHECK(sel_cmp_slt(x, y, i));
        CHECK(sel_u32((unsigned)x, (unsigned)y));
        int acc = x;
        tail_void(&acc);
        arms_void(y, &acc);
        CHECK(acc);
        CHECK((long long)(tail_fp((double)x, y & 7) * 4.0));
    }
    for (int i = 0; i < 8; i++) {
        L a = vl[i], b = vl[(i + 3) & 7], c = vl[(i + 5) & 7];
        CHECK(tail_l(a));
        CHECK(tail_rot6(a, b, c, a + 1, b - 1, c * 3));
        CHECK(tail_rev8(a, b, c, 4, 5, 6, a ^ b, 8));
        CHECK(tail_ind6(six, a, b, c, 2, 3));
        CHECK(stage5(a, b, c, i, a - c));
        CHECK(stage8(a, b, c, 4, 5, i, 7, b + c));
        CHECK(stage_mixed(a, b, c, i));
        CHECK(sel_add8_l(a, b));
        CHECK(sel_shl62(a, b));
        CHECK(sel_xor_l(a, b, c));
        CHECK(sel_cond_is_x(a));
        CHECK(sel_u64((UL)a, (UL)b));
        CHECK(sel_upper(a, c));
    }
    static const L counts[] = { -1, 0, 1, 7, 16 };
    for (int i = 0; i < 5; i++) {
        L n = counts[i];
        CHECK(sum_n(data, n));
        CHECK(sum_u(data, n < 0 ? 0u : (unsigned)n));
        CHECK(sum_incl(data, (int)n - 1));
        CHECK(sum_step2(data, (int)n));
        CHECK(sum_down(data, (int)n));
        CHECK(sum_ne(sdata, n < 0 ? 0 : (n > 8 ? 8 : n)));
    }
#if !defined(__clang__)
    for (int k = -2; k <= 2; k++)
        CHECK(nested_half(k));
#endif
#ifdef GEN
    return 0;
#else
    return n__ == (int)(sizeof expected / sizeof expected[0]) ? 0 : 255;
#endif
}
