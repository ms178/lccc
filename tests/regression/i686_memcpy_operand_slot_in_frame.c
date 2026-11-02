/* Pointer values that feed a small __builtin_memcpy get their stack slot
 * widened to the copy width.  A raw width (5, 12, 20, ...) used to be packed
 * at its exact size in the block-local slot pool while the final placement
 * rounded it up, so the second such pointer in a block was homed below the
 * emitted frame: i686 -O1 stored the source pointer at -4(%esp) and the
 * memcpy's own `pushl %esi` overwrote it (gcc.c-torture memcpy-a{1,2,4,8}.c).
 * Every function copies between two different globals and checks all bytes. */
typedef unsigned char u8;
typedef union { u8 v[88]; } buf_t;

buf_t src = {{ 1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15, 16,
              17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
              33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48 }};

#define CHECK_ONE(off, n)                                                    \
  static int __attribute__((noinline)) check_##off##_##n(void)              \
  {                                                                          \
    static buf_t dst = {{ [0 ... 87] = 0xaa }};                              \
    int i;                                                                   \
    __builtin_memcpy(dst.v + 8 + off, src.v + 8 + off, n);                   \
    __asm__("" ::: "memory");                                                \
    for (i = 0; i < 8 + off; i++)                                            \
      if (dst.v[i] != 0xaa) return 1;                                        \
    for (; i < 8 + off + n; i++)                                             \
      if (dst.v[i] != src.v[i]) return 2;                                    \
    for (; i < (int)sizeof(dst.v); i++)                                      \
      if (dst.v[i] != 0xaa) return 3;                                        \
    return 0;                                                                \
  }

CHECK_ONE(0, 5)
CHECK_ONE(1, 6)
CHECK_ONE(0, 7)
CHECK_ONE(3, 12)
CHECK_ONE(0, 15)
CHECK_ONE(2, 20)
CHECK_ONE(0, 31)

int main(void)
{
  int r = check_0_5() | check_1_6() | check_0_7() | check_3_12() |
          check_0_15() | check_2_20() | check_0_31();
  return r;
}
