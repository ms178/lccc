/* 64-bit __builtin_*_overflow on a 32-bit target.  The generic lowering
 * computed these in I128 (a u64 operand, or a multiply with a 64-bit operand
 * or result), which the i686 backend truncates to 64 bits: add/mul overflow
 * went unreported and the narrow-result forms ICEd ("wide i686 cast reached
 * the scalar cast emitter"; gcc.c-torture pr91450-1/-2, pr93494). */
typedef unsigned long long u64;
typedef long long s64;

#define N __attribute__((noipa))
N int uadd(u64 a, u64 b, u64 *r) { return __builtin_add_overflow(a, b, r); }
N int usub(u64 a, u64 b, u64 *r) { return __builtin_sub_overflow(a, b, r); }
N int umul(u64 a, u64 b, u64 *r) { return __builtin_mul_overflow(a, b, r); }
N int smul(s64 a, s64 b, s64 *r) { return __builtin_mul_overflow(a, b, r); }
N int smulll(s64 a, s64 b, s64 *r) { return __builtin_smulll_overflow(a, b, r); }
N int umulll(u64 a, u64 b, u64 *r) { return __builtin_umulll_overflow(a, b, r); }
N int mixmul(s64 a, u64 b, s64 *r) { return __builtin_mul_overflow(a, b, r); }
N int mixadd(s64 a, u64 b, s64 *r) { return __builtin_add_overflow(a, b, r); }
N int iimul(int a, int b, u64 *r) { return __builtin_mul_overflow(a, b, r); }
N int u16add(u64 a, u64 b, unsigned short *r) { return __builtin_add_overflow(a, b, r); }
N int s32mul(u64 a, s64 b, int *r) { return __builtin_mul_overflow(a, b, r); }
N int pmul(s64 a, s64 b) { return __builtin_mul_overflow_p(a, b, (s64)0); }
N int padd(u64 a, u64 b) { return __builtin_add_overflow_p(a, b, (u64)0); }

#define CHECK(call, ov, val)                                                 \
  do {                                                                       \
    if ((call) != (ov) || r != (val))                                        \
      return __LINE__;                                                       \
  } while (0)

N int run(void)
{
  { u64 r; CHECK(uadd(~0ULL, 1, &r), 1, 0); }
  { u64 r; CHECK(uadd(1ULL << 63, (1ULL << 63) - 1, &r), 0, ~0ULL); }
  { u64 r; CHECK(usub(0, 1, &r), 1, ~0ULL); }
  { u64 r; CHECK(umul(1ULL << 32, 1ULL << 32, &r), 1, 0); }
  { u64 r; CHECK(umul(0xFFFFFFFFULL, 0x100000001ULL, &r), 0, ~0ULL); }
  { u64 r; CHECK(umul(0x100000000ULL, 0xFFFFFFFFULL, &r), 0, 0xFFFFFFFF00000000ULL); }
  { u64 r; CHECK(umul(3, 0x5555555555555556ULL, &r), 1, 2); }
  { u64 r; CHECK(umulll(~0ULL, ~0ULL, &r), 1, 1); }
  { s64 r; CHECK(smul(1LL << 40, 1LL << 30, &r), 1, 0); }
  { s64 r; CHECK(smul(-(1LL << 31), 1LL << 32, &r), 0, (s64)(-(1LL << 31)) * (1LL << 32)); }
  { s64 r; CHECK(smul(-1, -0x7FFFFFFFFFFFFFFFLL - 1, &r), 1, -0x7FFFFFFFFFFFFFFFLL - 1); }
  { s64 r; CHECK(smulll(-3, 0x2AAAAAAAAAAAAAABLL, &r), 1, 0x7FFFFFFFFFFFFFFFLL); }
  { s64 r; CHECK(smulll(-3037000499LL, 3037000499LL, &r), 0, -9223372030926249001LL); }
  { s64 r; CHECK(mixmul(-1, 1, &r), 0, -1); }
  { s64 r; CHECK(mixmul(-1, 1ULL << 63, &r), 0, -0x7FFFFFFFFFFFFFFFLL - 1); }
  { s64 r; CHECK(mixmul(1, 1ULL << 63, &r), 1, -0x7FFFFFFFFFFFFFFFLL - 1); }
  { s64 r; CHECK(mixadd(-1, 1ULL << 63, &r), 0, 0x7FFFFFFFFFFFFFFFLL); }
  { s64 r; CHECK(mixadd(0, 1ULL << 63, &r), 1, -0x7FFFFFFFFFFFFFFFLL - 1); }
  { u64 r; CHECK(iimul(-1, 1, &r), 1, ~0ULL); }
  { u64 r; CHECK(iimul(65536, 65536, &r), 0, 1ULL << 32); }
  { unsigned short r; CHECK(u16add(0x10000, 0, &r), 1, 0); }
  { unsigned short r; CHECK(u16add(0xFFFF, 0, &r), 0, 0xFFFF); }
  { int r; CHECK(s32mul(0x80000000ULL, -1, &r), 0, (int)0x80000000); }
  { int r; CHECK(s32mul(0x80000000ULL, 1, &r), 1, (int)0x80000000); }
  if (pmul(1LL << 32, 1LL << 31) != 1 || pmul(1LL << 32, 1LL << 30) != 0)
    return __LINE__;
  if (padd(~0ULL, 1) != 1 || padd(~0ULL - 1, 1) != 0)
    return __LINE__;
  return 0;
}

int main(void)
{
  return run();
}
