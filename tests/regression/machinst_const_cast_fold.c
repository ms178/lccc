/* MachInst constant-cast fold (x86-64 MachInst path).
 *
 * gcc.c-torture/execute/20020219-1.c aborted at -O2/-O3/-Os: after IPCP
 * replaced `foo()` by `Copy %v, Const(I64(0x8000000000000000))`, the MachInst
 * lowering of the following `Cast I64->U64` picked the move width from the
 * DESTINATION signedness alone and emitted `movl $0` (the emitter truncates a
 * wide immediate to the 32-bit width).  The same lowering kept the 0x100 bit
 * of `Cast I32(300)->U8` and sign-extended `Cast U32(0xFFFFFFFF)->I64` to -1.
 *
 * Every accessor below is a constant-returning function so that IPCP feeds the
 * cast a constant operand late in the pipeline (after the IR constant folder
 * ran), which is exactly the shape that reached the backend.  Every expected
 * value is compared as a full-width integer so a stale/truncated register
 * image cannot pass by accident.  Exit status is the contract (torture style);
 * stdout is compared against GCC as well.  */
#include <stdio.h>
#include <stdlib.h>

#define NOINLINE __attribute__((noinline))

long long get_msb64(void) { return (long long)(1ULL << 63); }
int get_300(void) { return 300; }
unsigned get_u32_max(void) { return 0xFFFFFFFFu; }
signed char get_sc_m1(void) { return -1; }
short get_sh_m1(void) { return -1; }
unsigned short get_us_max(void) { return 0xFFFFu; }
long long get_m1_64(void) { return -1LL; }
int get_i32_min(void) { return (int)0x80000000; }
unsigned char get_uc_200(void) { return 200; }
long long get_mixed(void) { return 0x1234567890ABCDEFLL; }
unsigned long long get_umsb(void) { return 0x8000000000000001ULL; }


/* One function per case: the torture shape is `Cmp(cast(const), const)` feeding
 * a branch to abort(), in a function small enough for the MachInst path.
 * Each returns the case id on failure so the exit code identifies it.  */
static NOINLINE int t_i64_u64_msb(void) { if (((unsigned long long)get_msb64()) != (0x8000000000000000ULL)) return 1; return 0; }
static NOINLINE int t_i32_300_u8(void) { if (((unsigned char)get_300()) != (44)) return 2; return 0; }
static NOINLINE int t_i32_300_i8(void) { if (((long long)(signed char)get_300()) != (44)) return 3; return 0; }
static NOINLINE int t_u32max_u16(void) { if (((unsigned short)get_u32_max()) != (0xFFFF)) return 4; return 0; }
static NOINLINE int t_u32max_i16(void) { if (((long long)(short)get_u32_max()) != (-1LL)) return 5; return 0; }
static NOINLINE int t_mixed_u32(void) { if (((unsigned)get_mixed()) != (0x90ABCDEFu)) return 6; return 0; }
static NOINLINE int t_mixed_i32(void) { if (((long long)(int)get_mixed()) != ((long long)0xFFFFFFFF90ABCDEFULL)) return 7; return 0; }
static NOINLINE int t_mixed_u8(void) { if (((unsigned char)get_mixed()) != (0xEF)) return 8; return 0; }
static NOINLINE int t_umsb_i64(void) { if (((long long)get_umsb()) != ((long long)0x8000000000000001ULL)) return 9; return 0; }
static NOINLINE int t_u32max_i64(void) { if (((long long)get_u32_max()) != (0xFFFFFFFFLL)) return 10; return 0; }
static NOINLINE int t_u32max_u64(void) { if (((unsigned long long)get_u32_max()) != (0xFFFFFFFFULL)) return 11; return 0; }
static NOINLINE int t_sc_m1_u32(void) { if (((unsigned)get_sc_m1()) != (0xFFFFFFFFu)) return 12; return 0; }
static NOINLINE int t_sc_m1_u64(void) { if (((unsigned long long)get_sc_m1()) != (0xFFFFFFFFFFFFFFFFULL)) return 13; return 0; }
static NOINLINE int t_sh_m1_u64(void) { if (((unsigned long long)get_sh_m1()) != (0xFFFFFFFFFFFFFFFFULL)) return 14; return 0; }
static NOINLINE int t_us_max_i32(void) { if (((int)get_us_max()) != (0xFFFF)) return 15; return 0; }
static NOINLINE int t_us_max_i16(void) { if (((long long)(short)get_us_max()) != (-1LL)) return 16; return 0; }
static NOINLINE int t_uc200_i8(void) { if (((long long)(signed char)get_uc_200()) != (-56LL)) return 17; return 0; }
static NOINLINE int t_i32min_u64(void) { if (((unsigned long long)get_i32_min()) != (0xFFFFFFFF80000000ULL)) return 18; return 0; }
static NOINLINE int t_i32min_u32(void) { if (((unsigned)get_i32_min()) != (0x80000000u)) return 19; return 0; }
static NOINLINE int t_m1_64_u32_u64(void) { if (((unsigned long long)(unsigned)get_m1_64()) != (0xFFFFFFFFULL)) return 20; return 0; }
static NOINLINE int t_m1_64_i32_u64(void) { if (((unsigned long long)(int)get_m1_64()) != (0xFFFFFFFFFFFFFFFFULL)) return 21; return 0; }
static NOINLINE int t_m1_64_u16_i64(void) { if (((long long)(unsigned short)get_m1_64()) != (0xFFFFLL)) return 22; return 0; }
static NOINLINE int t_i64_u64_neg(void) { if (((unsigned long long)get_m1_64()) != (0xFFFFFFFFFFFFFFFFULL)) return 23; return 0; }
static NOINLINE int t_u64_i64_big(void) { if (((long long)get_umsb() < 0 ? 1 : 0) != (1)) return 24; return 0; }

int main(void) {
  int rc = 0;
  if ((rc = t_i64_u64_msb()) != 0) { printf("FAIL case %d (i64_u64_msb)\n", rc); return rc; }
  if ((rc = t_i32_300_u8()) != 0) { printf("FAIL case %d (i32_300_u8)\n", rc); return rc; }
  if ((rc = t_i32_300_i8()) != 0) { printf("FAIL case %d (i32_300_i8)\n", rc); return rc; }
  if ((rc = t_u32max_u16()) != 0) { printf("FAIL case %d (u32max_u16)\n", rc); return rc; }
  if ((rc = t_u32max_i16()) != 0) { printf("FAIL case %d (u32max_i16)\n", rc); return rc; }
  if ((rc = t_mixed_u32()) != 0) { printf("FAIL case %d (mixed_u32)\n", rc); return rc; }
  if ((rc = t_mixed_i32()) != 0) { printf("FAIL case %d (mixed_i32)\n", rc); return rc; }
  if ((rc = t_mixed_u8()) != 0) { printf("FAIL case %d (mixed_u8)\n", rc); return rc; }
  if ((rc = t_umsb_i64()) != 0) { printf("FAIL case %d (umsb_i64)\n", rc); return rc; }
  if ((rc = t_u32max_i64()) != 0) { printf("FAIL case %d (u32max_i64)\n", rc); return rc; }
  if ((rc = t_u32max_u64()) != 0) { printf("FAIL case %d (u32max_u64)\n", rc); return rc; }
  if ((rc = t_sc_m1_u32()) != 0) { printf("FAIL case %d (sc_m1_u32)\n", rc); return rc; }
  if ((rc = t_sc_m1_u64()) != 0) { printf("FAIL case %d (sc_m1_u64)\n", rc); return rc; }
  if ((rc = t_sh_m1_u64()) != 0) { printf("FAIL case %d (sh_m1_u64)\n", rc); return rc; }
  if ((rc = t_us_max_i32()) != 0) { printf("FAIL case %d (us_max_i32)\n", rc); return rc; }
  if ((rc = t_us_max_i16()) != 0) { printf("FAIL case %d (us_max_i16)\n", rc); return rc; }
  if ((rc = t_uc200_i8()) != 0) { printf("FAIL case %d (uc200_i8)\n", rc); return rc; }
  if ((rc = t_i32min_u64()) != 0) { printf("FAIL case %d (i32min_u64)\n", rc); return rc; }
  if ((rc = t_i32min_u32()) != 0) { printf("FAIL case %d (i32min_u32)\n", rc); return rc; }
  if ((rc = t_m1_64_u32_u64()) != 0) { printf("FAIL case %d (m1_64_u32_u64)\n", rc); return rc; }
  if ((rc = t_m1_64_i32_u64()) != 0) { printf("FAIL case %d (m1_64_i32_u64)\n", rc); return rc; }
  if ((rc = t_m1_64_u16_i64()) != 0) { printf("FAIL case %d (m1_64_u16_i64)\n", rc); return rc; }
  if ((rc = t_i64_u64_neg()) != 0) { printf("FAIL case %d (i64_u64_neg)\n", rc); return rc; }
  if ((rc = t_u64_i64_big()) != 0) { printf("FAIL case %d (u64_i64_big)\n", rc); return rc; }
  /* the torture shape verbatim: shift by (Y & 31) with Y = 32 */
  {
    long long C = 1ULL << 63, X;
    int Y = 32;
    X = C >> (Y & 31);
    if (X != (long long)(1ULL << 63)) abort();
  }
  puts("ok");
  return 0;
}
