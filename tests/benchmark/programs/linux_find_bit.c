/*
 * Workload-derived kernel: Linux 6.18.42, lib/find_bit.c and
 * include/asm-generic/bitops/__ffs.h.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 *
 * The `find_next_andnot_bit` kernel is a portable, standalone expansion of
 * Linux's FIND_NEXT_BIT(addr1[idx] & ~addr2[idx], ...) macro.  Kernel-only
 * headers, attributes, symbol exports, and statement-expression machinery are
 * intentionally absent.  The generic __ffs decision tree is retained so that
 * instruction selection can be inspected.  Full provenance is in
 * tests/benchmark/WORKLOAD_PROVENANCE.md.
 *
 * The sparse bitmap corpus makes each successful search cross many zero words;
 * this exposes branch layout, load scheduling, bit-scan lowering, and pointer
 * induction behavior without measuring kernel services.
 */
#include <stdio.h>

#define BITS_PER_LONG (sizeof(unsigned long) * 8UL)
#define WORD_COUNT (1UL << 14)
#define BIT_COUNT (WORD_COUNT * BITS_PER_LONG)
#define PASSES 1024U

static unsigned long linux_bitmap_a[WORD_COUNT];
static unsigned long linux_bitmap_b[WORD_COUNT];

/* Direct C equivalent from Linux's asm-generic/bitops/__ffs.h. */
static unsigned int
linux_generic_ffs(unsigned long word)
{
  unsigned int num = 0U;

  if ((word & 0xffffffffUL) == 0UL) {
    num += 32U;
    word >>= 32;
  }
  if ((word & 0xffffUL) == 0UL) {
    num += 16U;
    word >>= 16;
  }
  if ((word & 0xffUL) == 0UL) {
    num += 8U;
    word >>= 8;
  }
  if ((word & 0xfUL) == 0UL) {
    num += 4U;
    word >>= 4;
  }
  if ((word & 0x3UL) == 0UL) {
    num += 2U;
    word >>= 2;
  }
  if ((word & 0x1UL) == 0UL)
    num += 1U;
  return num;
}

/* Expansion of FIND_NEXT_BIT(addr1[idx] & ~addr2[idx], nop, ...). */
static unsigned long
linux_find_next_andnot_bit(const unsigned long *addr1,
                           const unsigned long *addr2,
                           unsigned long size, unsigned long start)
{
  unsigned long mask;
  unsigned long index;
  unsigned long value;
  unsigned long result = size;

  if (start >= size)
    return result;

  mask = ~0UL << (start % BITS_PER_LONG);
  index = start / BITS_PER_LONG;
  value = (addr1[index] & ~addr2[index]) & mask;

  while (!value) {
    if ((index + 1UL) * BITS_PER_LONG >= size)
      return result;
    index++;
    value = addr1[index] & ~addr2[index];
  }

  result = index * BITS_PER_LONG + linux_generic_ffs(value);
  return result < size ? result : size;
}

static void
make_sparse_bitmaps(void)
{
  unsigned long i;

  for (i = 0; i < WORD_COUNT; i++) {
    linux_bitmap_a[i] = 0UL;
    linux_bitmap_b[i] = ~0UL;
    /* One searchable bit roughly every 64 words. */
    if ((i & 63UL) == 5UL) {
      unsigned long bit = (i * 13UL + 7UL) & (BITS_PER_LONG - 1UL);
      linux_bitmap_a[i] = 1UL << bit;
      linux_bitmap_b[i] = 0UL;
    }
  }
}

static int
check_kernel(void)
{
  unsigned long first[3];
  unsigned long second[3];

  first[0] = 1UL << 5;
  first[1] = 1UL << 2;
  first[2] = 0UL;
  second[0] = 0UL;
  second[1] = ~0UL;
  second[2] = ~0UL;

  if (linux_find_next_andnot_bit(first, second, 3UL * BITS_PER_LONG, 0UL)
      != 5UL)
    return 0;
  if (linux_find_next_andnot_bit(first, second, 3UL * BITS_PER_LONG, 6UL)
      != 3UL * BITS_PER_LONG)
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
  make_sparse_bitmaps();

  for (pass = 0; pass < PASSES; pass++) {
    unsigned long offset = (unsigned long)(pass & 63U);
    unsigned long bit;
    do {
      bit = linux_find_next_andnot_bit(linux_bitmap_a, linux_bitmap_b,
                                       BIT_COUNT, offset);
      if (bit < BIT_COUNT) {
        checksum ^= bit + ((unsigned long)pass << 19);
        offset = bit + 1UL;
      }
    } while (bit < BIT_COUNT);

    /* Change an input word between scans without changing bitmap bounds. */
    {
      unsigned long word = (((unsigned long)pass * 13UL) & 255UL) * 64UL + 5UL;
      linux_bitmap_b[word] ^= 1UL << (((unsigned long)pass * 7UL)
                                      & (BITS_PER_LONG - 1UL));
    }
  }

  printf("%lu\n", checksum);
  return 0;
}
