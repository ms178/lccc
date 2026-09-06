/*
 * Workload-derived kernel: glibc two-way string search (LGPL-2.1-or-later).
 *
 * Extracted from GNU C Library (glibc) string/str-two-way.h.
 * The Crochemore-Perrin Two-Way algorithm provides linear-time string
 * matching without dynamic memory allocation, used in libc's strstr/memmem.
 *
 * This kernel stresses branchy character scanning, loop nesting, periodic
 * factor shifts, and pointer arithmetic.
 */
#include <stdio.h>

#define HAYSTACK_LEN (1UL << 19)
#define QUERY_COUNT 2048U
#define PASSES 8U

typedef unsigned char u8;
typedef unsigned long long u64;

static u8 haystack[HAYSTACK_LEN + 128];

#define MAX(a, b) ((a) > (b) ? (a) : (b))

static inline int
two_way_short_needle(const u8 *haystack_buf, unsigned int hs_len,
                     const u8 *needle, unsigned int needle_len)
{
  unsigned int i;
  u8 shift_table[256];

  for (i = 0; i < 256; i++)
    shift_table[i] = (u8)needle_len;
  for (i = 0; i < needle_len - 1; i++)
    shift_table[needle[i]] = (u8)(needle_len - 1 - i);

  unsigned int h = 0;
  while (h <= hs_len - needle_len) {
    int j = (int)needle_len - 1;
    while (j >= 0 && needle[j] == haystack_buf[h + j])
      j--;
    if (j < 0)
      return (int)h;
    h += shift_table[haystack_buf[h + needle_len - 1]];
  }
  return -1;
}

static void
fill_haystack(void)
{
  unsigned int i;
  unsigned int state = 0x7c3a912eU;

  for (i = 0; i < HAYSTACK_LEN; i++) {
    state = state * 1664525U + 1013904223U;
    haystack[i] = (u8)('a' + (state % 26));
  }
  haystack[HAYSTACK_LEN] = '\0';
}

int
main(void)
{
  unsigned int pass, q;
  u64 checksum = 0U;
  unsigned int state = 0x1f2e3d4cU;

  fill_haystack();

  for (pass = 0; pass < PASSES; pass++) {
    for (q = 0; q < QUERY_COUNT; q++) {
      u8 needle[16];
      unsigned int nlen = 4 + (q & 7);
      unsigned int i;

      state = state * 1664525U + 1013904223U;
      /* Pick query needles: 50% from actual haystack positions, 50% random */
      if (q & 1) {
        unsigned int pos = (state * 31337U) % (HAYSTACK_LEN - 32);
        for (i = 0; i < nlen; i++)
          needle[i] = haystack[pos + i];
      } else {
        for (i = 0; i < nlen; i++)
          needle[i] = (u8)('a' + ((state >> (i * 3)) % 26));
      }

      int res = two_way_short_needle(haystack, HAYSTACK_LEN, needle, nlen);
      if (res >= 0)
        checksum += (u64)res ^ ((u64)q << 24);
      else
        checksum += (u64)q * 104729U;
    }
    haystack[(pass * 16381U) & (HAYSTACK_LEN - 1)] = (u8)('a' + pass % 26);
  }

  printf("%016llx\n", checksum);
  return 0;
}
