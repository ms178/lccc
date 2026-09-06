/*
 * Workload-derived kernel: Zstandard (ZSTD) match counting / ZSTD_count
 * (BSD-3-Clause / GPL-2.0).
 *
 * Extracted from ZSTD lib/compress/zstd_compress_internal.h (ZSTD_count).
 * Match length calculation is the inner hotspot of Zstd, LZ4, Snappy, and
 * fast LZ77 encoders.
 *
 * This kernel stresses 64-bit unaligned memory comparisons, trailing-zero
 * count instruction selection (tzcnt/bsf), pointer increments, and short-loop
 * branch prediction.
 */
#include <stdio.h>

#define BUFFER_SIZE (1UL << 20)
#define QUERY_COUNT (1U << 17)
#define PASSES 16U

typedef unsigned char u8;
typedef unsigned int u32;
typedef unsigned long long u64;

static u8 buffer[BUFFER_SIZE + 64];

static inline u64
read64(const void *p)
{
  u64 v;
  __builtin_memcpy(&v, p, sizeof(v));
  return v;
}

static inline unsigned int
zstd_count(const u8 *pIn, const u8 *pMatch, const u8 *pInLimit)
{
  const u8 *const pStart = pIn;
  const u8 *const pInLoopLimit = pInLimit - (sizeof(u64) - 1);

  while (pIn < pInLoopLimit) {
    u64 const diff = read64(pMatch) ^ read64(pIn);
    if (!diff) {
      pIn += sizeof(u64);
      pMatch += sizeof(u64);
      continue;
    }
    pIn += __builtin_ctzll(diff) >> 3;
    return (unsigned int)(pIn - pStart);
  }

  while ((pIn < pInLimit) && (*pIn == *pMatch)) {
    pIn++;
    pMatch++;
  }
  return (unsigned int)(pIn - pStart);
}

static void
fill_buffer(void)
{
  unsigned int i;
  u32 state = 0x85467329U;

  for (i = 0; i < BUFFER_SIZE; i++) {
    state = state * 1664525U + 1013904223U;
    /* Create repeating patterns mixed with noise to yield realistic matches */
    if ((state & 0x07U) == 0 && i >= 64)
      buffer[i] = buffer[i - 64 + (state >> 28)];
    else
      buffer[i] = (u8)(state >> 24);
  }
}

int
main(void)
{
  unsigned int pass, q;
  u64 checksum = 0U;
  u32 state = 0x13579bdfU;

  fill_buffer();

  for (pass = 0; pass < PASSES; pass++) {
    for (q = 0; q < QUERY_COUNT; q++) {
      unsigned int offset1, offset2, limit_len;
      unsigned int count;

      state = state * 1664525U + 1013904223U;
      offset1 = state & (BUFFER_SIZE - 256 - 1);
      offset2 = (state >> 12) & (BUFFER_SIZE - 256 - 1);
      limit_len = 16 + (state & 0x7fU);

      count = zstd_count(buffer + offset1, buffer + offset2,
                         buffer + offset1 + limit_len);
      checksum += ((u64)count << ((q & 7) * 8)) ^ ((u64)offset1);
    }
  }

  printf("%016llx\n", checksum);
  return 0;
}
