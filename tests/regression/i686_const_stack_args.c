/* i686: a CONSTANT call argument must be stored straight into the outgoing
 * stack slot, and the four bytes stored must be exactly the bytes the
 * historical `operand_to_eax` + `movl %eax, N(%esp)` pair produced.
 *
 * `store_const_to_stack_arg` replaces that pair with a single
 * `movl $imm, N(%esp)`.  Two things can go wrong, and both are observable
 * here rather than only in the assembly:
 *
 *   * a per-kind convention mismatch.  i64/i128 truncate to their low half,
 *     `F32`/`F64` pass the IEEE bit pattern, `D32`/`D64` the BID pattern's low
 *     half, `LongDouble` its leading four bytes, `I8`/`I16` sign-extend.  A
 *     wrong arm shows up as a wrong checksum, not as a crash.
 *   * wrong slot offsets.  The folded store writes an explicit displacement,
 *     so an off-by-one in the outgoing-area bookkeeping lands an argument in a
 *     neighbour's slot.  That is why 8-byte arguments (`long long`, `double`)
 *     are interleaved with the 4-byte ones even though they travel a different
 *     path: they shift every following offset.
 *
 * Non-constant arguments -- a variable, an alloca ADDRESS, a global address, a
 * function pointer -- are interleaved so the `%eax` path that must remain is
 * exercised inside the same call, and a zero constant is passed in every
 * position that can hold one: zero is deliberately NOT folded (its historical
 * form is the 2-byte `xorl %eax, %eax`, and folding it would cost 8 bytes to
 * save one instruction), so those arguments must still arrive correctly
 * through the old sequence.
 *
 * Reference values below are gcc -m32 -O2, which agrees at -O0/-O1/-O2/-O3. */
#include <stdio.h>
#include <string.h>

/* Reference values: gcc -m32 at -O0/-O1/-O2/-O3 (all four agree).  Build with
 * -DPRINT_REF to re-derive them instead of checking against them. */
#ifndef EXPECT_A
#define EXPECT_A 5911436263486334280ull
#define EXPECT_B 825493310136ull
#define EXPECT_V 7669537u
#endif

static unsigned g_blob[4] = {0x11111111u, 0x22222222u, 0x33333333u, 0x44444444u};

__attribute__((noinline)) static int add7(int x) { return x + 7; }

/* Twelve arguments.  cdecl on i686 passes every one of them on the stack, so
 * each is an outgoing stack argument: constants of several kinds, the two
 * 8-byte kinds that shift the offsets, and four operands that genuinely need
 * `%eax` (a variable, an alloca address, a global address, a function
 * pointer). */
__attribute__((noinline)) static unsigned long long
mix(int zero, int neg_min, signed char c, short s, long long ll, float f,
    float fnegzero, double d, double dnegzero, int *alloca_addr,
    unsigned *global_addr, int (*fp)(int), int var) {
  unsigned long long acc = 0ull;
  unsigned u;

  acc = acc * 31ull + (unsigned)zero;
  acc = acc * 31ull + (unsigned)neg_min;
  acc = acc * 31ull + (unsigned)(int)c;
  acc = acc * 31ull + (unsigned)(int)s;
  acc = acc * 31ull + (unsigned long long)ll;

  memcpy(&u, &f, sizeof u);        /* F32 travels as its IEEE bit pattern */
  acc = acc * 31ull + u;
  memcpy(&u, &fnegzero, sizeof u); /* -0.0f is 0x80000000, NOT a zero imm */
  acc = acc * 31ull + u;

  memcpy(&u, &d, sizeof u);        /* low half of the double's bit pattern */
  acc = acc * 31ull + u;
  memcpy(&u, &dnegzero, sizeof u);
  acc = acc * 31ull + u;

  acc = acc * 31ull + (unsigned)alloca_addr[1];
  acc = acc * 31ull + (unsigned)global_addr[2];
  acc = acc * 31ull + (unsigned)fp(var);
  acc = acc * 31ull + (unsigned)var;
  return acc;
}

/* A run of eight 4-byte arguments, constant and variable interleaved: this is
 * the shape where the folded stores must keep their displacements in step. */
__attribute__((noinline)) static unsigned
eight(int a, int b, int c, int d, int e, int f, int g, int h) {
  return (unsigned)a * 3u + (unsigned)b * 5u + (unsigned)c * 7u +
         (unsigned)d * 11u + (unsigned)e * 13u + (unsigned)f * 17u +
         (unsigned)g * 19u + (unsigned)h * 23u;
}

int main(void) {
  int local[4] = {10, 20, 30, 40};
  int var = 5;
  unsigned long long a, b;
  unsigned v = 0;
  int k;

  a = mix(0, -2147483647 - 1, (signed char)-1, (short)-2,
          0x0123456789abcdefLL, 1.5f, -0.0f, 2.25, -0.0, local, g_blob, add7,
          var);
  b = mix(0, 0, (signed char)0, (short)0, 0LL, 0.0f, 0.0f, 0.0, 0.0, local,
          g_blob, add7, 0);

  v = eight(11, 0, -1, 0x7fffffff, var, -2147483647 - 1, 0, 12345);
  for (k = 0; k < 3; k++) {
    v = v * 3u + eight(k, 0, -k, k * 7, var, 1, 0, -1);
  }

  /* The variadic path stages stack arguments through the same code. */
  printf("varargs %d %d %d %d\n", 7, -9, 0, 0x7fffffff);

#ifdef PRINT_REF
  printf("REF a=%llu b=%llu v=%u\n", a, b, v);
  return 0;
#endif

  if (a != EXPECT_A || b != EXPECT_B || v != EXPECT_V) {
    printf("FAIL i686_const_stack_args a=%llu b=%llu v=%u\n"
           "     want a=%llu b=%llu v=%u\n",
           a, b, v, (unsigned long long)EXPECT_A, (unsigned long long)EXPECT_B,
           (unsigned)EXPECT_V);
    return 1;
  }
  printf("PASS i686_const_stack_args a=%llu b=%llu v=%u\n", a, b, v);
  return 0;
}
