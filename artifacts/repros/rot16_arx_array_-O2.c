/*
 * The same P0 -O2 miscompile, in the form that first exposed it: a local
 * u32 x[16] whose fields are hand-indexed (the shape produced by the
 * aggregate_sroa constant-offset split, form 4).  The point of keeping this
 * variant next to the pure-scalar ones is that here we can prove the fault is
 * in CODEGEN, not in the IR: the final IR for this function, evaluated by
 * hand, yields gcc's value, while the emitted binary yields a different one.
 *
 *   $CCC -O2 this.c -o /tmp/a   ->  ... out[5] = 1667510585 ...
 *   gcc  -O2 this.c -o /tmp/g   ->  ... out[5] = 1667511609 ...
 *   CCC_NO_AGGREGATE_SPLIT=1    ->  1667511609  (correct: frame slots keep
 *                                                 the values out of the spill
 *                                                 path, so the RA bug hides)
 *
 * IR-level trace of the ON build (instructions of `core` after the whole IR
 * pipeline, x[5] chain): v284 = rotl7(v228 ^ v263), out[5] = v284 + in[5];
 * every operand matches the C source, and evaluating that chain gives
 * 1667511609.  So the IR is right and the miscompile is introduced by
 * register allocation / frame lowering.  See rot16-arx-O2-miscompile.md.
 */
#include <stdio.h>

typedef unsigned int u32;
#define R(v, n) (((v) << (n)) | ((v) >> (32 - (n))))
#define QR(a, b, c, d)                                                      \
  do {                                                                      \
    a += b;                                                                 \
    d ^= a;                                                                 \
    d = R(d, 16);                                                           \
    c += d;                                                                 \
    b ^= c;                                                                 \
    b = R(b, 12);                                                           \
    a += b;                                                                 \
    d ^= a;                                                                 \
    d = R(d, 8);                                                            \
    c += d;                                                                 \
    b ^= c;                                                                 \
    b = R(b, 7);                                                            \
  } while (0)

__attribute__((noinline)) static void
core(u32 out[16], const u32 in[16])
{
  u32 x[16] = { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16 };
  x[5] = in[5];
  x[9] = in[9];
  x[12] = in[12];
  x[13] = in[13];
  QR(x[0], x[4], x[8], x[12]);
  QR(x[1], x[5], x[9], x[13]);
  QR(x[0], x[4], x[8], x[12]);
  out[3] = x[3] + in[3];
  out[5] = x[5] + in[5];
  out[9] = x[9] + in[9];
  out[12] = x[12] + in[12];
  out[13] = x[13] + in[13];
}

static const u32 TIN[16] = { 1, 2, 3, 4, 5, 6, 7, 8,
                              9, 10, 11, 12, 13, 14, 15, 16 };

int
main(void)
{
  u32 o[16] = { 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 };
  core(o, TIN);
  for (int i = 0; i < 16; i++)
    printf("%u ", o[i]);
  printf("\n");
  return o[5] != 1667511609u;
}
