/*
 * Same P0 -O2 miscompile as rot16_arx_scalar_-O2_min.c, in its original
 * 16-scalar / three-quarter-round form (the shape this session started from).
 * The array spelling — rot16_arx_array_-O2.c — produces the identical wrong
 * value even though it takes a completely different route through the IR
 * pipeline (aggregate SROA), and the pure-scalar form here never touches an
 * aggregate at all.  That is how we know the fault is not in aggregate_sroa:
 * both forms land in the same register-reuse/spill bug in codegen.
 *
 * gcc -O2 (and lccc -O0/-O1) print out[5] = 1667511609; lccc -O2/-O3/-Os
 * print 1667510585 (low by 1024).  Positions 9, 12, 13 agree.
 */
#include <stdio.h>

typedef unsigned int u32;
#define R(v, n) (((v) << (n)) | ((v) >> (32 - (n))))
#define QRR(a, b, c, d)                                                     \
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
  u32 x0 = 1, x1 = 2, x2 = 3, x3 = 4, x4 = 5, x5 = 6, x6 = 7, x7 = 8;
  u32 x8 = 9, x9 = 10, x10 = 11, x11 = 12, x12 = 13, x13 = 14, x14 = 15,
      x15 = 16;
  x5 = in[5];
  x9 = in[9];
  x12 = in[12];
  x13 = in[13];
  QRR(x0, x4, x8, x12);
  QRR(x1, x5, x9, x13);
  QRR(x0, x4, x8, x12);
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
  if (o[5] != 1667511609u) {
    printf("MISCOMPILED out[5]=%u (want 1667511609)\n", o[5]);
    return 1;
  }
  printf("ok\n");
  return 0;
}
