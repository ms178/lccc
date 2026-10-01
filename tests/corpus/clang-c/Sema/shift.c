// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/shift.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/shift.c" 2


# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h" 1
# 25 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h"
# 1 "/usr/include/limits.h" 1 3 4
# 26 "/usr/include/limits.h" 3 4
# 1 "/usr/include/features.h" 1 3 4
# 345 "/usr/include/features.h" 3 4
# 1 "/usr/include/stdc-predef.h" 1 3 4
# 346 "/usr/include/features.h" 2 3 4
# 375 "/usr/include/features.h" 3 4
# 1 "/usr/include/sys/cdefs.h" 1 3 4
# 392 "/usr/include/sys/cdefs.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 393 "/usr/include/sys/cdefs.h" 2 3 4
# 376 "/usr/include/features.h" 2 3 4
# 399 "/usr/include/features.h" 3 4
# 1 "/usr/include/gnu/stubs.h" 1 3 4
# 10 "/usr/include/gnu/stubs.h" 3 4
# 1 "/usr/include/gnu/stubs-64.h" 1 3 4
# 11 "/usr/include/gnu/stubs.h" 2 3 4
# 400 "/usr/include/features.h" 2 3 4
# 27 "/usr/include/limits.h" 2 3 4
# 144 "/usr/include/limits.h" 3 4
# 1 "/usr/include/bits/posix1_lim.h" 1 3 4
# 160 "/usr/include/bits/posix1_lim.h" 3 4
# 1 "/usr/include/bits/local_lim.h" 1 3 4
# 38 "/usr/include/bits/local_lim.h" 3 4
# 1 "/usr/include/linux/limits.h" 1 3 4
# 39 "/usr/include/bits/local_lim.h" 2 3 4
# 161 "/usr/include/bits/posix1_lim.h" 2 3 4
# 145 "/usr/include/limits.h" 2 3 4



# 1 "/usr/include/bits/posix2_lim.h" 1 3 4
# 149 "/usr/include/limits.h" 2 3 4
# 26 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h" 2
# 4 "Sema/shift.c" 2



enum {
  X = 1 << 0,
  Y = 1 << 1,
  Z = 1 << 2
};

void test(void) {
  char c;

  c = 0 << 0;
  c = 0 << 1;
  c = 1 << 0;
  c = 1 << -0;
  c = 1 >> -0;
  c = 1 << -1;
  c = 1 >> -1;
  c = 1 << (unsigned)-1;

  c = 1 >> (unsigned)-1;
  c = 1 << c;
  c <<= 0;
  c >>= 0;
  c <<= 1;
  c >>= 1;
  c <<= -1;
  c >>= -1;
  c <<= 999999;
  c >>= 999999;
  c <<= 8;
  c >>= 8;
  c <<= 8 +1;
  c >>= 8 +1;
  (void)((long)c << 8);

  int i;
  i = 1 << ((sizeof(int) * 8) - 2);
  i = 2 << ((sizeof(int) * 8) - 1);
  i = 1 << ((sizeof(int) * 8) - 1);
  i = -1 << ((sizeof(int) * 8) - 1);
  i = -1 << 0;
  i = 0 << ((sizeof(int) * 8) - 1);
  i = (char)1 << ((sizeof(int) * 8) - 2);

  unsigned u;
  u = 1U << ((sizeof(int) * 8) - 1);
  u = 5U << ((sizeof(int) * 8) - 1);

  long long int lli;
  lli = (-2147483647 -1) << 2;
  lli = 1LL << (sizeof(long long) * 8 - 2);
}



enum { b = (0 << 8) };


void test_pr5544(void) {
  (void) (((1) > 63 && (1) < 128 ? (((unsigned long long) 1)<<((1)-64)) : (unsigned long long) 0));
}

void test_shift_too_much(char x) {
  if (0)
    (void) (x >> 80);
  (void) (x >> 80);
}

typedef unsigned vec16 __attribute__((vector_size(16)));
typedef unsigned vec8 __attribute__((vector_size(8)));

void vect_shift_1(vec16 *x) { *x = *x << 4; }

void vect_shift_2(vec16 *x, vec16 y) { *x = *x << y; }

void vect_shift_3(vec16 *x, vec8 y) {
  *x = *x << y;
}
