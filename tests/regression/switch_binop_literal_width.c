// Regression: switch controlling expression `int OP literal` must be
// compared at 32-bit width.
//
// get_expr_type() reports the widened *storage* type of an int-op-literal
// expression (I64 on LP64, because integer literals are I64 at that level),
// while lower_arithmetic_binop() computes the value as a 32-bit `addl`.
// lower_switch_stmt() typed the Switch terminator with the former, so the
// dispatch chain emitted `cmpq $-1, %rdi`: the upper 32 bits of %rdi are
// ABI-undefined for an `int` argument (SysV passes only the low 32 bits)
// and are ZERO after any 32-bit ALU op, so `case -1:` never matched and
// large positive garbage could match other cases.  Found by
// tests/stress/run_stress.py (switch family, every seed, every -O level).
//
// Every shape below must agree with GCC; the driver compares stdout and exit
// status against gcc.  `noinline` keeps the callee's argument in a register
// whose upper half the caller deliberately pollutes via a 64-bit value.
#include <stdint.h>
#include <stdio.h>

static __attribute__((noinline)) int sw_add0(int x) {
    switch (x + 0) { case -1: return 1; case 5: return 2; case -65536: return 3; }
    return 0;
}
static __attribute__((noinline)) int sw_add1(int x) {
    switch (x + 1) { case -1: return 1; case 5: return 2; case 2147483647: return 3; }
    return 0;
}
static __attribute__((noinline)) int sw_mul1(int x) {
    switch (x * 1) { case -1: return 1; case 5: return 2; case -2147483647 - 1: return 3; }
    return 0;
}
static __attribute__((noinline)) int sw_sub(int x) {
    switch (x - 3) { case -1: return 1; case 5: return 2; case -2: return 3; }
    return 0;
}
static __attribute__((noinline)) int sw_or(int x) {
    switch (x | 0) { case -1: return 1; case 5: return 2; default: return 9; }
}
static __attribute__((noinline)) int sw_xor(int x) {
    switch (x ^ 1) { case -2: return 1; case 4: return 2; }
    return 0;
}
static __attribute__((noinline)) int sw_neg(int x) {
    switch (-x + 0) { case 1: return 1; case -5: return 2; }
    return 0;
}
static __attribute__((noinline)) int sw_short(short x) {
    switch (x + 0) { case -1: return 1; case 5: return 2; case -32768: return 3; }
    return 0;
}
static __attribute__((noinline)) int sw_uchar(unsigned char x) {
    switch (x - 1) { case -1: return 1; case 254: return 2; }
    return 0;
}
static __attribute__((noinline)) int sw_uint(unsigned x) {
    switch (x + 1u) { case 0u: return 1; case 5u: return 2; case 0x80000000u: return 3; }
    return 0;
}
static __attribute__((noinline)) int sw_long(long x) {
    // 64-bit controlling expression must stay 64-bit.
    switch (x + 0) { case -1L: return 1; case 0x100000000L: return 2; case 5L: return 3; }
    return 0;
}
static __attribute__((noinline)) int sw_cmp(int x) {
    // Comparison result is an int; `(x < 0) + 0` is I32.
    switch ((x < 0) + 0) { case 0: return 10; case 1: return 11; }
    return 0;
}

// Pollute the upper halves: pass values derived from a 64-bit variable so
// the caller has no reason to zero-extend.
static volatile int64_t wide = (int64_t)0xFFFFFFFF00000000LL;

int main(void) {
    int64_t w = wide;
    int m1 = (int)(w | 0xFFFFFFFFu);          // -1 with garbage above
    int p5 = (int)(w | 5);                    // 5 with garbage above
    int big = (int)(w | 0x7FFFFFFF);
    unsigned long long h = 1469598103934665603ull;
#define H(v) h = (h ^ (unsigned long long)(v)) * 1099511628211ull
    H(sw_add0(m1)); H(sw_add0(p5)); H(sw_add0(-65536)); H(sw_add0(7));
    H(sw_add1(m1 - 1)); H(sw_add1(4)); H(sw_add1(big - 1)); H(sw_add1(0));
    H(sw_mul1(m1)); H(sw_mul1(p5)); H(sw_mul1(-2147483647 - 1)); H(sw_mul1(0));
    H(sw_sub(2)); H(sw_sub(8)); H(sw_sub(1)); H(sw_sub(3));
    H(sw_or(m1)); H(sw_or(p5)); H(sw_or(0));
    H(sw_xor(m1)); H(sw_xor(p5)); H(sw_xor(0));
    H(sw_neg(m1)); H(sw_neg(p5)); H(sw_neg(2));
    H(sw_short(-1)); H(sw_short(5)); H(sw_short(-32768)); H(sw_short(0));
    H(sw_uchar(0)); H(sw_uchar(255)); H(sw_uchar(1));
    H(sw_uint(0xFFFFFFFFu)); H(sw_uint(4)); H(sw_uint(0x7FFFFFFFu)); H(sw_uint(9));
    H(sw_long(-1)); H(sw_long(0x100000000L)); H(sw_long(5)); H(sw_long(0));
    H(sw_cmp(m1)); H(sw_cmp(p5)); H(sw_cmp(0));
    printf("%d %d %d %d %d %d %d %d %d %d %d %d\n",
           sw_add0(m1), sw_add0(p5), sw_add1(m1 - 1), sw_mul1(m1), sw_sub(2), sw_or(m1),
           sw_xor(m1), sw_neg(m1), sw_short(-1), sw_uchar(0), sw_uint(0xFFFFFFFFu), sw_cmp(m1));
    printf("%llx\n", h);
    return (sw_add0(m1) == 1 && sw_add0(p5) == 2 && sw_add1(m1 - 1) == 1 && sw_mul1(m1) == 1
            && sw_or(m1) == 1 && sw_xor(m1) == 1 && sw_neg(m1) == 1 && sw_cmp(m1) == 11) ? 0 : 1;
}
