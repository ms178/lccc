/*
 * Workload-derived kernel: glibc commit 7cba77790f3279bec3ac20e9c7632b021cd53f95,
 * string/memcmp.c, aligned-word fast path.
 *
 * SPDX-License-Identifier: LGPL-2.1-or-later
 *
 * This is a defined-behavior standalone adaptation of glibc's
 * memcmp_common_alignment strategy: compare four native words per iteration,
 * then determine ordering bytewise on a little-endian target.  The upstream
 * implementation uses integer-address arithmetic and platform headers to
 * support arbitrary alignments; this benchmark deliberately supplies aligned
 * word arrays and omits the unrelated unaligned path.  Provenance and exact
 * source pin are in tests/benchmark/WORKLOAD_PROVENANCE.md.
 *
 * The mismatch moves through the input on every pass.  That exposes bulk-load
 * selection, unrolling, early-exit branches, byte extraction, and live-range
 * pressure without accidentally benchmarking the host libc's memcmp.
 */
#include <stdio.h>

#define WORD_COUNT (1UL << 13)
#define PASSES 4096U

static unsigned long glibc_left[WORD_COUNT];
static unsigned long glibc_right[WORD_COUNT];

/* glibc's little-endian CMP_LT_OR_GT path compares the differing word bytes. */
static int
glibc_memcmp_bytes(unsigned long a, unsigned long b)
{
  const unsigned char *pa = (const unsigned char *)&a;
  const unsigned char *pb = (const unsigned char *)&b;
  unsigned long i;

  for (i = 0UL; i < sizeof(unsigned long); i++) {
    if (pa[i] != pb[i])
      return (int)pa[i] - (int)pb[i];
  }
  return 0;
}

/* Safe pointer-form adaptation of glibc's common-alignment four-word loop. */
static int
glibc_memcmp_common_alignment(const unsigned long *src1,
                              const unsigned long *src2,
                              unsigned long words)
{
  while (words >= 4UL) {
    unsigned long a0 = src1[0];
    unsigned long a1 = src1[1];
    unsigned long a2 = src1[2];
    unsigned long a3 = src1[3];
    unsigned long b0 = src2[0];
    unsigned long b1 = src2[1];
    unsigned long b2 = src2[2];
    unsigned long b3 = src2[3];

    if (a0 != b0)
      return glibc_memcmp_bytes(a0, b0);
    if (a1 != b1)
      return glibc_memcmp_bytes(a1, b1);
    if (a2 != b2)
      return glibc_memcmp_bytes(a2, b2);
    if (a3 != b3)
      return glibc_memcmp_bytes(a3, b3);

    src1 += 4;
    src2 += 4;
    words -= 4UL;
  }

  while (words) {
    if (*src1 != *src2)
      return glibc_memcmp_bytes(*src1, *src2);
    src1++;
    src2++;
    words--;
  }
  return 0;
}

static void
make_inputs(void)
{
  unsigned long i;
  unsigned long state = 0x9e3779b97f4a7c15UL;

  for (i = 0UL; i < WORD_COUNT; i++) {
    state ^= state << 7;
    state ^= state >> 9;
    state ^= state << 8;
    glibc_left[i] = state;
    glibc_right[i] = state;
  }
}

static int
check_kernel(void)
{
  unsigned long left[4];
  unsigned long right[4];

  left[0] = 0UL;
  left[1] = 0x0102030405060708UL;
  left[2] = ~0UL;
  left[3] = 7UL;
  right[0] = left[0];
  right[1] = left[1];
  right[2] = left[2];
  right[3] = left[3];
  if (glibc_memcmp_common_alignment(left, right, 4UL) != 0)
    return 0;
  right[1] = 0x0102030405060709UL;
  if (glibc_memcmp_common_alignment(left, right, 4UL) >= 0)
    return 0;
  return 1;
}

int
main(void)
{
  unsigned int pass;
  unsigned long checksum = 0UL;

  if (!check_kernel())
    return 2;
  make_inputs();

  for (pass = 0; pass < PASSES; pass++) {
    unsigned long word = ((unsigned long)pass * 4051UL) & (WORD_COUNT - 1UL);
    unsigned long flip = 1UL << (((unsigned long)pass * 11UL)
                                 & ((sizeof(unsigned long) * 8UL) - 1UL));
    int result;

    glibc_right[word] = glibc_left[word] ^ flip;
    result = glibc_memcmp_common_alignment(glibc_left, glibc_right, WORD_COUNT);
    checksum += (unsigned long)(result + 257) * ((unsigned long)pass + 1UL);
    glibc_right[word] = glibc_left[word];
  }

  printf("%lu\n", checksum);
  return 0;
}
