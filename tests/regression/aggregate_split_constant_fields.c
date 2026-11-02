/*
 * aggregate_sroa form 4 (constant-offset splitting) behaviour lock.
 *
 * Every case computes a result twice: once through an aggregate (the shape the
 * split targets) and once through plain scalars (the shape it cannot touch).
 * The two must agree, so the test fails if a split either miscompiles or is
 * applied where the soundness contract says it must not be. The "must not"
 * cases (volatile access, overlapping views) are exercised precisely because
 * refusing them has to be *decided* by the analysis rather than by luck.
 *
 * The loop-carried case is deliberate: form 3 refuses blocks with a back edge
 * (it rewrites a single block at a time), while form 4 rewires every access of
 * the object and so must work across a loop.
 *
 * exit 0 = pass (the runner's convention for tests/regression/*.c).
 */
#include <stdio.h>

typedef unsigned int u32;
typedef unsigned long long u64;

typedef struct {
  u32 a, b, c, d;
} V;

static int failures;

static void
check(const char *name, u32 got, u32 want)
{
  if (got != want) {
    printf("FAIL %-24s got %u want %u\n", name, got, want);
    failures++;
  } else {
    printf("ok   %-24s %u\n", name, got);
  }
}

/* (a) field-wise stores/loads on a local struct, initialized by a whole-struct
 * copy — the copy-in the split has to expand rather than refuse. */
static u32
aggregate_fields(V seed)
{
  V t;
  t = seed;
  t.a = t.a + 1u;
  t.b = t.b ^ t.a;
  t.c = t.a + t.b;
  t.d = t.c * 3u;
  t.a ^= t.d;
  return t.a + t.b + t.c + t.d;
}

static u32
scalar_fields(V seed)
{
  u32 a = seed.a, b = seed.b, c, d;
  a = a + 1u;
  b = b ^ a;
  c = a + b;
  d = c * 3u;
  a ^= d;
  return a + b + c + d;
}

/* (b) loop-carried state in a local array, all constant subscripts. */
static u32
aggregate_loop(u32 k)
{
  u32 x[4];
  int i;
  x[0] = k;
  x[1] = k ^ 0x9e37u;
  x[2] = k + 5u;
  x[3] = k * 7u;
  for (i = 0; i < 8; i++) {
    x[0] += x[1];
    x[2] ^= x[0];
    x[1] = (x[1] << 3) | (x[1] >> 29);
    x[3] += x[2] ^ x[0];
  }
  return x[0] + x[1] + x[2] + x[3];
}

static u32
scalar_loop(u32 k)
{
  u32 x0 = k, x1 = k ^ 0x9e37u, x2 = k + 5u, x3 = k * 7u;
  int i;
  for (i = 0; i < 8; i++) {
    x0 += x1;
    x2 ^= x0;
    x1 = (x1 << 3) | (x1 >> 29);
    x3 += x2 ^ x0;
  }
  return x0 + x1 + x2 + x3;
}

/* (c) one volatile access into the object: the whole object must be refused,
 * because the volatile read/write is not ours to move or retype. */
static u32
mixed_volatile(u32 *slot, u32 k)
{
  u32 v;
  volatile u32 *vv = (volatile u32 *)&v;
  v = k;
  v = v + 3u;
  *vv = *vv ^ 0xf0f0u;
  v = v * 5u;
  *slot = v;
  return v + *vv;
}

static u32
mixed_volatile_oracle(u32 k)
{
  u32 v = k;
  v = v + 3u;
  v = v ^ 0xf0f0u;
  v = v * 5u;
  return v + v;
}

#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
/* (d) two views of the same bytes: a u32 at +0 and a u64 at +0 overlap, so the
 * offsets are not independent storage and the split must refuse the object. */
static u32
overlapping_views(u32 k)
{
  u32 w[2];
  u64 *wide = (u64 *)w;
  w[0] = k;
  w[1] = k ^ 7u;
  *wide = *wide + 0x100000001ull;
  return w[0] ^ w[1];
}

static u32
overlapping_views_oracle(u32 k)
{
  u32 lo = k, hi = k ^ 7u;
  u64 v = (((u64)hi) << 32) | (u64)lo;
  v = v + 0x100000001ull;
  return (u32)v ^ (u32)(v >> 32);
}
#endif

int
main(void)
{
  V seed = { 3u, 11u, 7u, 5u };
  u32 slot = 0u;
  u32 acc_a = 0u, acc_s = 0u;
  int i;

  check("struct fields", aggregate_fields(seed), scalar_fields(seed));
  for (i = 0; i < 64; i++) {
    acc_a += aggregate_loop((u32)i * 2654435761u);
    acc_s += scalar_loop((u32)i * 2654435761u);
  }
  check("array loop-carried", acc_a, acc_s);
  check("volatile in object", mixed_volatile(&slot, 12345u),
        mixed_volatile_oracle(12345u));
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
  check("overlapping views", overlapping_views(4000000000u),
        overlapping_views_oracle(4000000000u));
#endif
  printf("%s\n", failures ? "FAILURES" : "ALL PASS");
  return failures != 0;
}
