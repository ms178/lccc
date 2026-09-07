/*
 * P0 MISCOMPILE at -O2 (and -O3/-Os): ARX / rotate chains with high scalar
 * register pressure compute a wrong value.  No aggregates, no arrays, no
 * pointers into local memory: 9 live `u32` locals, straight-line adds/xors/
 * rotates.  Found while landing the aggregate_sroa constant-offset split
 * (form 4): the split reproduces the same wrong answer only because it
 * *removes* the frame slots (everything becomes SSA values), which pushes the
 * function over the spill path.  Verified pre-existing: this file miscompiles
 * at upstream HEAD with zero local changes.
 *
 *   ./target/fastbuild/lccc -O2 rot16_arx_scalar_-O2_min.c -o /tmp/m
 *   gcc -O2                    rot16_arx_scalar_-O2_min.c -o /tmp/g
 *
 * Expected (gcc, and the lccc -O0/-O1 result):
 *   0 0 0 8 0 3147014 20 ...
 * lccc -O2:
 *   out[5] is low by exactly 1024 (bit 10).
 *
 * Root cause, traced from the emitted assembly of `core` (see the .md next to
 * this file):
 *
 *   leal 2(%r8), %eax      ; x1 = x1 + x5
 *   movl %eax, 28(%rsp)    ; spill x1
 *   ...
 *   addl 28(%rsp), %r8d    ; x5 + x1  -> result left in %r8d, which HELD x5
 *   movl %r8d, 16(%rsp)    ; store x1
 *   xorl 16(%rsp), %r14d   ; x13 ^= x1        (correct, reads the slot)
 *   xorl %r11d, %r8d       ; x5 ^= x9         (WRONG: %r8d is now x1, not x5)
 *
 * The value produced for `x1` is computed *into the register that still holds
 * `x5`*, so `x5`'s next use (the rotl7) reads 24584 (x1) instead of 24576
 * (x5): rotl7(24584^10)=3145984 rather than rotl7(24576^10)=3147008 -> out[5]
 * differs by exactly 1024.
 *
 * This is NOT a register-allocation fault.  `LCCC_NO_PEEPHOLE=1` shows the
 * allocator emitting a legal sequence -- `movl %r8d, %eax; addl 28(%rsp), %eax;
 * movl %eax, 16(%rsp)` -- which the asm peephole `fold_accumulator_alu_store`
 * then folds into `addl 28(%rsp), %r8d; movl %r8d, 16(%rsp)`, clobbering the
 * live `%r8`.  The fold's "is the source register dead?" scan accepted
 * `xorl %r11d, %r8d` as a redefinition because the classifier reports `%r8` as
 * that instruction's destination, and an x86 two-address ALU op reads its
 * destination too.  Fixed by demanding a full, value-independent redefinition
 * (`pure_family_write`); see artifacts/repros/rot16-arx-O2-miscompile.md and
 * the unit tests in `acc_fold_src_kill_tests`.
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
  u32 o[16] = { 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 };
  core(o, TIN);
  /* Self-check: exit 0 iff the values are the ones gcc/clang/-O0 produce. */
  if (o[3] != 8u || o[5] != 3147014u) {
    printf("MISCOMPILED out[3]=%u out[5]=%u (want 8 3147014)\n", o[3], o[5]);
    return 1;
  }
  printf("ok\n");
  return 0;
}
