// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23: --c17
# 1 "C/C23/n3029.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 444 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/C23/n3029.c" 2



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
# 26 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h" 2
# 5 "C/C23/n3029.c" 2
# 21 "C/C23/n3029.c"
enum x {
a = 2147483647,
b = (9223372036854775807LL*2ULL+1ULL),

a_type = _Generic(a, char: 1, unsigned char: 2, signed char: 3, short: 4, unsigned short: 5, int: 6, unsigned int: 7, long: 8, unsigned long: 9, long long: 10, unsigned long long: 11, default: 0xFF ),
b_type = _Generic(b, char: 1, unsigned char: 2, signed char: 3, short: 4, unsigned short: 5, int: 6, unsigned int: 7, long: 8, unsigned long: 9, long long: 10, unsigned long long: 11, default: 0xFF )
};

_Static_assert(_Generic(a, char: 1, unsigned char: 2, signed char: 3, short: 4, unsigned short: 5, int: 6, unsigned int: 7, long: 8, unsigned long: 9, long long: 10, unsigned long long: 11, default: 0xFF ) == _Generic(b, char: 1, unsigned char: 2, signed char: 3, short: 4, unsigned short: 5, int: 6, unsigned int: 7, long: 8, unsigned long: 9, long long: 10, unsigned long long: 11, default: 0xFF ), "ok");

extern enum x e_a;
extern __typeof(b) e_a;
extern __typeof(a) e_a;

enum a {
  a0 = 0xFFFFFFFFFFFFFFFFULL

};

_Bool e (void) {
  return a0;
}

int f (void) {
  return a0;

}

unsigned long g (void) {
  return a0;
}

unsigned long long h (void) {
  return a0;
}

enum big_enum {
  big_enum_a = 9223372036854775807L,

  big_enum_b = a + 1,

  big_enum_c = (9223372036854775807LL*2ULL+1ULL)

};

_Static_assert(_Generic(big_enum_a, char: 1, unsigned char: 2, signed char: 3, short: 4, unsigned short: 5, int: 6, unsigned int: 7, long: 8, unsigned long: 9, long long: 10, unsigned long long: 11, default: 0xFF ) == _Generic(big_enum_b, char: 1, unsigned char: 2, signed char: 3, short: 4, unsigned short: 5, int: 6, unsigned int: 7, long: 8, unsigned long: 9, long long: 10, unsigned long long: 11, default: 0xFF ), "ok");
