/*
 * Workload-derived kernel: LZ4 block compression inner loop (BSD-2-Clause).
 *
 * Adapted from Yann Collet's LZ4 lib/lz4.c (LZ4_compress_fast).
 * LZ4 compression is widely used in filesystems (ZFS, btrfs), Linux zram,
 * databases, and game engines.
 *
 * This kernel exercises 4-byte hash table lookups, unaligned word memory
 * reads, match-length extension loops, literal run output generation, and
 * store-to-load forwarding.
 */
#include <stdio.h>

#define SRC_SIZE (1UL << 19)
#define HASH_LOG 14
#define HASH_SIZE (1U << HASH_LOG)
#define PASSES 24U

typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

static u8 src_data[SRC_SIZE + 64];
static u8 dst_data[SRC_SIZE * 2];
static u32 hash_table[HASH_SIZE];

static inline u32
read32(const void *p)
{
  u32 v;
  __builtin_memcpy(&v, p, sizeof(v));
  return v;
}

static inline u32
hash4(u32 val)
{
  return (val * 2654435761U) >> (32 - HASH_LOG);
}

static unsigned int
lz4_compress_block(const u8 *src, unsigned int src_len, u8 *dst)
{
  const u8 *ip = src;
  const u8 *const iend = src + src_len;
  const u8 *const mflimit = iend - 12;
  const u8 *anchor = src;
  u8 *op = dst;
  unsigned int i;

  for (i = 0; i < HASH_SIZE; i++)
    hash_table[i] = 0;

  if (src_len < 13)
    return 0;

  ip++;
  while (ip < mflimit) {
    u32 h = hash4(read32(ip));
    const u8 *ref = src + hash_table[h];
    hash_table[h] = (u32)(ip - src);

    if (ref < ip && ref >= src && read32(ref) == read32(ip)) {
      /* Match found! Encode literals then match length */
      unsigned int lit_len = (unsigned int)(ip - anchor);
      const u8 *match = ref + 4;
      ip += 4;

      while (ip < iend && *ip == *match) {
        ip++;
        match++;
      }

      unsigned int match_len = (unsigned int)(match - ref - 4);
      u8 *token = op++;

      /* Literals */
      if (lit_len >= 15) {
        *token = (15 << 4);
        unsigned int l = lit_len - 15;
        while (l >= 255) { *op++ = 255; l -= 255; }
        *op++ = (u8)l;
      } else {
        *token = (u8)(lit_len << 4);
      }
      for (i = 0; i < lit_len; i++)
        *op++ = anchor[i];

      /* Offset */
      u16 offset = (u16)(ip - match);
      *op++ = (u8)(offset & 0xff);
      *op++ = (u8)(offset >> 8);

      /* Match length */
      if (match_len >= 15) {
        *token |= 15;
        unsigned int m = match_len - 15;
        while (m >= 255) { *op++ = 255; m -= 255; }
        *op++ = (u8)m;
      } else {
        *token |= (u8)match_len;
      }

      anchor = ip;
    } else {
      ip += 1 + ((ip - anchor) >> 6);
    }
  }

  /* Trailing literals */
  unsigned int last_lits = (unsigned int)(iend - anchor);
  *op++ = (u8)(last_lits < 15 ? (last_lits << 4) : (15 << 4));
  for (i = 0; i < last_lits; i++)
    *op++ = anchor[i];

  return (unsigned int)(op - dst);
}

static void
fill_source(void)
{
  unsigned int i;
  u32 state = 0x5a1b3c7dU;

  for (i = 0; i < SRC_SIZE; i++) {
    state = state * 1664525U + 1013904223U;
    if ((state & 0x0fU) < 6 && i >= 128)
      src_data[i] = src_data[i - 128 + (state & 0x3fU)];
    else
      src_data[i] = (u8)(state >> 24);
  }
}

int
main(void)
{
  unsigned int pass;
  u64 checksum = 0U;

  fill_source();

  for (pass = 0; pass < PASSES; pass++) {
    unsigned int out_len = lz4_compress_block(src_data, SRC_SIZE, dst_data);
    checksum += ((u64)out_len << 32) ^ dst_data[0] ^ ((u64)dst_data[out_len / 2] << 16);
    src_data[(pass * 4099U) & (SRC_SIZE - 1)] ^= 0x55U;
  }

  printf("%016llx\n", checksum);
  return 0;
}
