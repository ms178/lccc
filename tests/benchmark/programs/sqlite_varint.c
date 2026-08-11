/*
 * Workload-derived kernel: SQLite 3.53.4, src/util.c
 * (sqlite3PutVarint and sqlite3GetVarint).
 *
 * SQLite is in the public domain.  This file retains the production fast-path
 * decoder shape and adds a deterministic standalone corpus/harness.  Project
 * types, assertions, and unrelated SQLite interfaces were replaced with C
 * built-in types only.  Full provenance is recorded in
 * tests/benchmark/WORKLOAD_PROVENANCE.md.
 *
 * This is a branch- and shift-heavy database hot-path kernel: the corpus
 * deliberately contains values requiring every one of SQLite's 1..9-byte
 * encodings.
 */
#include <stdio.h>

#define VALUE_COUNT (1U << 18)
#define PASSES 24U
#define SLOT_2_0 0x001fc07fU
#define SLOT_4_2_0 0xf01fc07fU

typedef unsigned char u8;
typedef unsigned int u32;
typedef unsigned long long u64;

static unsigned char sqlite_varint_bytes[VALUE_COUNT * 9U];
static unsigned int sqlite_varint_offsets[VALUE_COUNT];
static unsigned int sqlite_varint_used;

static int
sqlite_put_varint(unsigned char *p, u64 v)
{
  int i;
  int j;
  int n;
  u8 buf[10];

  if (v <= 0x7fU) {
    p[0] = (u8)(v & 0x7fU);
    return 1;
  }
  if (v <= 0x3fffU) {
    p[0] = (u8)(((v >> 7) & 0x7fU) | 0x80U);
    p[1] = (u8)(v & 0x7fU);
    return 2;
  }
  if (v & 0xff00000000000000ULL) {
    p[8] = (u8)v;
    v >>= 8;
    for (i = 7; i >= 0; i--) {
      p[i] = (u8)((v & 0x7fU) | 0x80U);
      v >>= 7;
    }
    return 9;
  }

  n = 0;
  do {
    buf[n++] = (u8)((v & 0x7fU) | 0x80U);
    v >>= 7;
  } while (v != 0U);
  buf[0] &= 0x7fU;
  for (i = 0, j = n - 1; j >= 0; j--, i++)
    p[i] = buf[j];
  return n;
}

/* Production fast-path decoder adapted verbatim in algorithmic structure. */
static u8
sqlite_get_varint(const unsigned char *p, u64 *v)
{
  u32 a;
  u32 b;
  u32 s;

  if (((const signed char *)p)[0] >= 0) {
    *v = *p;
    return 1;
  }
  if (((const signed char *)p)[1] >= 0) {
    *v = ((u32)(p[0] & 0x7fU) << 7) | p[1];
    return 2;
  }

  a = ((u32)p[0]) << 14;
  b = p[1];
  p += 2;
  a |= *p;
  if (!(a & 0x80U)) {
    a &= SLOT_2_0;
    b &= 0x7fU;
    b <<= 7;
    a |= b;
    *v = a;
    return 3;
  }

  a &= SLOT_2_0;
  p++;
  b <<= 14;
  b |= *p;
  if (!(b & 0x80U)) {
    b &= SLOT_2_0;
    a <<= 7;
    a |= b;
    *v = a;
    return 4;
  }

  b &= SLOT_2_0;
  s = a;
  p++;
  a <<= 14;
  a |= *p;
  if (!(a & 0x80U)) {
    b <<= 7;
    a |= b;
    s >>= 18;
    *v = ((u64)s << 32) | a;
    return 5;
  }

  s <<= 7;
  s |= b;
  p++;
  b <<= 14;
  b |= *p;
  if (!(b & 0x80U)) {
    a &= SLOT_2_0;
    a <<= 7;
    a |= b;
    s >>= 18;
    *v = ((u64)s << 32) | a;
    return 6;
  }

  p++;
  a <<= 14;
  a |= *p;
  if (!(a & 0x80U)) {
    a &= SLOT_4_2_0;
    b &= SLOT_2_0;
    b <<= 7;
    a |= b;
    s >>= 11;
    *v = ((u64)s << 32) | a;
    return 7;
  }

  a &= SLOT_2_0;
  p++;
  b <<= 14;
  b |= *p;
  if (!(b & 0x80U)) {
    b &= SLOT_4_2_0;
    a <<= 7;
    a |= b;
    s >>= 4;
    *v = ((u64)s << 32) | a;
    return 8;
  }

  p++;
  a <<= 15;
  a |= *p;
  b &= SLOT_2_0;
  b <<= 8;
  a |= b;
  s <<= 4;
  b = p[-4];
  b &= 0x7fU;
  b >>= 3;
  s |= b;
  *v = ((u64)s << 32) | a;
  return 9;
}

static u64
next_value(unsigned int index, unsigned int state)
{
  switch (index % 9U) {
  case 0:
    return (u64)(state & 0x7fU);
  case 1:
    return 0x80U | (u64)(state & 0x3fffU);
  case 2:
    return 0x4000U | (u64)(state & 0x1fffffU);
  case 3:
    return 0x200000U | (u64)(state & 0x0fffffffU);
  case 4:
    return 0x10000000ULL | ((u64)state << 3);
  case 5:
    return 0x800000000ULL | ((u64)state << 10);
  case 6:
    return 0x40000000000ULL | ((u64)state << 17);
  case 7:
    return 0x2000000000000ULL | ((u64)state << 24);
  default:
    return 0x8000000000000000ULL | ((u64)state << 25) | index;
  }
}

static void
make_corpus(void)
{
  unsigned int i;
  unsigned int state = 0x6d2b79f5U;
  unsigned int used = 0U;

  for (i = 0; i < VALUE_COUNT; i++) {
    u64 value;
    state = state * 1664525U + 1013904223U;
    value = next_value(i, state);
    sqlite_varint_offsets[i] = used;
    used += (unsigned int)sqlite_put_varint(sqlite_varint_bytes + used, value);
  }
  sqlite_varint_used = used;
}

static int
check_known_values(void)
{
  static const u64 values[] = {
    0U, 0x7fU, 0x80U, 0x3fffU, 0x4000U, 0x1fffffU,
    0x200000U, 0xfffffffU, 0x10000000ULL, 0x7fffffffffffffffULL,
    0xffffffffffffffffULL
  };
  unsigned char buffer[9];
  unsigned int i;

  for (i = 0; i < sizeof(values) / sizeof(values[0]); i++) {
    u64 decoded = 0U;
    int written = sqlite_put_varint(buffer, values[i]);
    u8 read = sqlite_get_varint(buffer, &decoded);
    if ((int)read != written || decoded != values[i])
      return 0;
  }
  return 1;
}

int
main(void)
{
  unsigned int pass;
  u64 checksum = 0U;

  if (!check_known_values())
    return 2;
  make_corpus();

  for (pass = 0; pass < PASSES; pass++) {
    unsigned int i;
    for (i = 0; i < VALUE_COUNT; i++) {
      u64 value = 0U;
      u8 n = sqlite_get_varint(sqlite_varint_bytes + sqlite_varint_offsets[i],
                                &value);
      checksum += value ^ ((u64)n << (i & 31U));
    }
    /* Flip a payload bit only; continuation bits and offsets stay valid. */
    sqlite_varint_bytes[sqlite_varint_offsets[(pass * 104729U)
                                               & (VALUE_COUNT - 1U)]] ^= 1U;
  }

  if (sqlite_varint_used == 0U)
    return 3;
  printf("%llx\n", checksum);
  return 0;
}
