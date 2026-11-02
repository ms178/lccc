// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "Sema/warn-cast-qual.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/warn-cast-qual.c" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h" 1
# 56 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h"
# 1 "/usr/include/stdint.h" 1 3 4
# 25 "/usr/include/stdint.h" 3 4
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
# 26 "/usr/include/stdint.h" 2 3 4
# 1 "/usr/include/bits/wchar.h" 1 3 4
# 22 "/usr/include/bits/wchar.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 23 "/usr/include/bits/wchar.h" 2 3 4
# 27 "/usr/include/stdint.h" 2 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 28 "/usr/include/stdint.h" 2 3 4








typedef signed char int8_t;
typedef short int int16_t;
typedef int int32_t;

typedef long int int64_t;







typedef unsigned char uint8_t;
typedef unsigned short int uint16_t;

typedef unsigned int uint32_t;



typedef unsigned long int uint64_t;
# 65 "/usr/include/stdint.h" 3 4
typedef signed char int_least8_t;
typedef short int int_least16_t;
typedef int int_least32_t;

typedef long int int_least64_t;






typedef unsigned char uint_least8_t;
typedef unsigned short int uint_least16_t;
typedef unsigned int uint_least32_t;

typedef unsigned long int uint_least64_t;
# 90 "/usr/include/stdint.h" 3 4
typedef signed char int_fast8_t;

typedef long int int_fast16_t;
typedef long int int_fast32_t;
typedef long int int_fast64_t;
# 103 "/usr/include/stdint.h" 3 4
typedef unsigned char uint_fast8_t;

typedef unsigned long int uint_fast16_t;
typedef unsigned long int uint_fast32_t;
typedef unsigned long int uint_fast64_t;
# 119 "/usr/include/stdint.h" 3 4
typedef long int intptr_t;


typedef unsigned long int uintptr_t;
# 134 "/usr/include/stdint.h" 3 4
typedef long int intmax_t;
typedef unsigned long int uintmax_t;
# 57 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdint.h" 2
# 6 "Sema/warn-cast-qual.c" 2

void foo(void) {
  const char *const ptr = 0;
  const char *const *ptrptr = 0;
  char *const *ptrcptr = 0;
  char **ptrptr2 = 0;
  char *y = (char *)ptr;
  char **y1 = (char **)ptrptr;
  const char **y2 = (const char **)ptrptr;
  char *const *y3 = (char *const *)ptrptr;
  const char **y4 = (const char **)ptrcptr;

  char *z = (char *)(uintptr_t)(const void *)ptr;
  char *z1 = (char *)(const void *)ptr;

  volatile char *vol = 0;
  char *vol2 = (char *)vol;
  const volatile char *volc = 0;
  char *volc2 = (char *)volc;

  int **intptrptr;
  const int **intptrptrc = (const int **)intptrptr;
  volatile int **intptrptrv = (volatile int **)intptrptr;

  int *intptr;
  const int *intptrc = (const int *)intptr;

  const char **charptrptrc;
  char **charptrptr = (char **)charptrptrc;

  const char *constcharptr;
  char *charptr = (char *)constcharptr;
  const char *constcharptr2 = (char *)constcharptr;
  const char *charptr2 = (char *)charptr;
# 73 "Sema/warn-cast-qual.c"
}

void bar_0(void) {
  struct C {
    const int a;
    int b;
  };

  const struct C S = {0, 0};

  *(int *)(&S.a) = 0;
  *(int *)(&S.b) = 0;






}

void bar_1(void) {
  struct C {
    const int a;
    int b;
  };

  struct C S = {0, 0};
  S.b = 0;

  *(int *)(&S.a) = 0;
  *(int *)(&S.b) = 0;






}
