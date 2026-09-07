/*
 * Locks the soundness condition of the x86-64 asm peephole
 * `fold_accumulator_alu_store` (src/backend/x86/codegen/peephole/passes/
 * local_patterns.rs), found by differential testing of the aggregate_sroa
 * constant-offset split (form 4) but pre-existing and independent of it.
 *
 * The pass folds
 *
 *     movl %REGd, %eax
 *     addl OFFSET(%rsp), %eax
 *     movl %eax, OFFSET2(%rsp)
 *
 * into `addl OFFSET(%rsp), %REGd ; movl %REGd, OFFSET2(%rsp)`, which DESTROYS
 * %REGd. That is only legal when nothing reads %REGd again, and the old kill
 * test asked whether %REGd was a line's classified *destination* -- a question
 * an x86 two-address ALU op answers with a lie: `xorl %r11d, %r8d` writes %r8
 * *and reads it*. The fold fired anyway and a later consumer of %r8 read the
 * clobbered value. Symptom on this shape: bit 10 lost, `out[5]` = 3145990
 * instead of 3147014 (rotl7(24584^10) instead of rotl7(24576^10)).
 *
 * The arithmetic is the shrunk repro verbatim: nine live u32 locals,
 * straight-line add/xor/rotate chains, results stored through a pointer
 * parameter so several values are simultaneously spilled.
 *
 * If the register allocator ever stops handing this function the spill pattern
 * the fold matched, the test keeps passing but stops exercising the pass. To
 * check the fold is still being considered:
 *
 *     ./target/fastbuild/lccc -O2 -S tests/regression/peephole_acc_fold_arx_src_kill.c -o /tmp/s.s
 *     # look for `addl N(%rsp), %Rd` followed by `movl %Rd, M(%rsp)` with a
 *     # later read of %Rd, and compare CCC_NO_PEEPHOLE_PHASE1=1 output
 *
 * The unit tests in `acc_fold_src_kill_tests` (same file as the pass) pin the
 * legality rule itself, independently of any allocation decision.
 */
#include <stdio.h>

typedef unsigned int u32;
#define R(v, n) (((v) << (n)) | ((v) >> (32 - (n))))

__attribute__((noinline)) static void
core(u32 out[16], const u32 in[16])
{
  u32 x0 = 1;
  u32 x1 = 2;
  u32 x3 = 4;
  u32 x4 = 5;
  u32 x5 = 6;
  u32 x8 = 9;
  u32 x9 = 10;
  u32 x12 = 13;
  u32 x13 = 14;
  x5 = in[5];
  x9 = in[9];
  x12 = in[12];
  x13 = in[13];
  x8 += x12;
  x4 ^= x8;
  x0 += x4;
  x12 ^= x0;
  x8 += x12;
  x4 ^= x8;
  x1 += x5;
  x5 = R(x5, 12);
  x1 += x5;
  x13 ^= x1;
  x5 ^= x9;
  x5 = R(x5, 7);
  x12 ^= x0;
  x4 ^= x8;
  x0 += x4;
  x12 ^= x0;
  out[3] = x3 + in[3];
  out[5] = x5 + in[5];
  out[9] = x9 + in[9];
  out[12] = x12 + in[12];
  out[13] = x13 + in[13];
}

static const u32 TIN[16] = { 1, 2, 3, 4, 5, 6, 7, 8,
                             9, 10, 11, 12, 13, 14, 15, 16 };

int
main(void)
{
  /* Reference: gcc -O0/-O1/-O2 and lccc -O0/-O1 all agree on these. */
  static const u32 expect[16] = { 0u, 0u, 0u, 8u, 0u, 3147014u, 0u, 0u,
                                   0u, 20u, 0u, 0u, 55u, 24596u, 0u, 0u };
  u32 o[16] = { 0u };
  int i, bad = 0;
  core(o, TIN);
  for (i = 0; i < 16; i++) {
    if (o[i] != expect[i]) {
      printf("MISMATCH out[%d]=%u want %u\n", i, o[i], expect[i]);
      bad = 1;
    }
  }
  if (bad)
    return 1;
  printf("ok\n");
  return 0;
}
