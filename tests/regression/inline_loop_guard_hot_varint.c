/*
 * Loop-guard inliner: two hot callers of a wide, acyclic static decoder.
 * The varint encoder/decoder follows SQLite 3.53.4 (public domain); this
 * standalone harness exercises every 1..9 byte length and checks two loop
 * traversals against an independent recurrence on the original values.
 *
 * By default, hot_forward and hot_backward are exported yet each has one
 * call from this TU's main. With LOOP_GUARD_STATIC_WRAPPERS, they are static
 * and each has one call from an exported service instead. Neither one-call
 * shape proves a caller is cold: other TUs may invoke an exported service
 * repeatedly. The guard tier must clone both decoder sites at -O2, keep the
 * decoder outlined at -Os, and preserve the result at every optimization
 * level. The structural companion builds and checks both shapes.
 */
/* A libc-free mode also permits executable checks on cross targets. */
#ifndef LOOP_GUARD_FREESTANDING
#include <stdio.h>
#endif

#define SLOT_2_0 0x001fc07fU
#define SLOT_4_2_0 0xf01fc07fU

typedef unsigned char u8;
typedef unsigned int u32;
typedef unsigned long long u64;

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

static const u64 values[] = {
    0ULL, 0x7fULL, 0x80ULL, 0x3fffULL, 0x4000ULL,
    0x200000ULL, 0x10000000ULL, 0x800000000ULL,
    0x40000000000ULL, 0x2000000000000ULL,
    0x100000000000000ULL, 0x7fffffffffffffffULL,
    0xffffffffffffffffULL
};
#define COUNT (sizeof(values) / sizeof(values[0]))
#define REPS 99U

#ifdef LOOP_GUARD_STATIC_WRAPPERS
#define HOT_LINKAGE static
#else
#define HOT_LINKAGE
#endif

HOT_LINKAGE __attribute__((noinline)) u64 hot_forward(const u8 encoded[][9]) {
    u64 sum = 0;
    for (unsigned int pass = 0; pass < REPS; pass++) {
        for (unsigned int i = 0; i < COUNT; i++) {
            u64 got = 0;
            u8 n = sqlite_get_varint(encoded[i], &got);
            sum += got ^ ((u64)n << (i & 31U));
        }
    }
    return sum;
}

HOT_LINKAGE __attribute__((noinline)) u64 hot_backward(const u8 encoded[][9]) {
    u64 sum = 0;
    for (unsigned int pass = 0; pass < REPS; pass++) {
        for (unsigned int i = COUNT; i != 0; i--) {
            u64 got = 0;
            u8 n = sqlite_get_varint(encoded[i - 1], &got);
            sum ^= got + ((u64)n << ((i - 1U) & 31U));
            sum = (sum << 3) | (sum >> 61);
        }
    }
    return sum;
}

#ifdef LOOP_GUARD_STATIC_WRAPPERS
/* Each helper is static but reached through an exported root, not just main. */
__attribute__((noinline)) u64 service_forward(const u8 encoded[][9]) {
    return hot_forward(encoded);
}
__attribute__((noinline)) u64 service_backward(const u8 encoded[][9]) {
    return hot_backward(encoded);
}
#define RUN_FORWARD service_forward
#define RUN_BACKWARD service_backward
#else
#define RUN_FORWARD hot_forward
#define RUN_BACKWARD hot_backward
#endif

int main(void) {
    u8 encoded[COUNT][9] = {{0}};
    u8 lengths[COUNT];
    unsigned int seen_lengths = 0;
    for (unsigned int i = 0; i < COUNT; i++) {
        lengths[i] = (u8)sqlite_put_varint(encoded[i], values[i]);
        if (lengths[i] < 1U || lengths[i] > 9U) return 1;
        seen_lengths |= 1U << (lengths[i] - 1U);
    }
    if (seen_lengths != (1U << 9) - 1U) return 2;
    u64 expected_forward = 0;
    u64 expected_backward = 0;
    for (unsigned int pass = 0; pass < REPS; pass++) {
        for (unsigned int i = 0; i < COUNT; i++)
            expected_forward += values[i] ^ ((u64)lengths[i] << (i & 31U));
        for (unsigned int i = COUNT; i != 0; i--) {
            expected_backward ^= values[i - 1U] +
                ((u64)lengths[i - 1U] << ((i - 1U) & 31U));
            expected_backward = (expected_backward << 3) | (expected_backward >> 61);
        }
    }
    const u64 forward = RUN_FORWARD((const u8 (*)[9])encoded);
    const u64 backward = RUN_BACKWARD((const u8 (*)[9])encoded);
    if (forward != expected_forward || backward != expected_backward) {
#ifndef LOOP_GUARD_FREESTANDING
        printf("fail: %llx/%llx %llx/%llx\n", forward, expected_forward,
               backward, expected_backward);
#endif
        return 1;
    }
#ifndef LOOP_GUARD_FREESTANDING
    printf("%llx %llx\n", forward, backward);
#endif
    return 0;
}
