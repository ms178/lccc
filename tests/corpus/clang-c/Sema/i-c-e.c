// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c99
# 1 "Sema/i-c-e.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 411 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/i-c-e.c" 2


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
# 4 "Sema/i-c-e.c" 2
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h" 1
# 25 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h"
# 1 "/usr/include/limits.h" 1 3 4
# 26 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h" 2
# 5 "Sema/i-c-e.c" 2

int a(void) {int p; *(1 ? &p : (void*)(0 && (a(),1))) = 10;}



int expr;
char w[__builtin_constant_p(expr) ? expr : 1];

char v[sizeof(__builtin_constant_p(0)) == sizeof(int) ? 1 : -1];

int implicitConversion = 1.0;
char floatArith[(int)(1.0+2.0)];



char b[__builtin_constant_p((int)(1.0+2.0)) ? (int)(1.0+2.0) : -1];

struct c {
  int a : (
           __builtin_constant_p((int)(1.0+2.0)) ? (int)(1.0+
     expr
           ) : -1);
};



void PR49239(void) {
  goto check_not_vla;
  char not_vla[__builtin_constant_p(1) ? ({ 42; }) : -1];
check_not_vla:;
  _Static_assert(sizeof(not_vla) == 42, "");





  goto check_also_not_vla;
  char also_not_vla[__builtin_expect(({ 76; }), 0)];
check_also_not_vla:;
  _Static_assert(sizeof(also_not_vla) == 76, "");
}


void test1(int n, int* p) { *(n ? p : (void *)(7-7)) = 1; }
void test2(int n, int* p) { *(n ? p : (void *)0) = 1; }



char array[1024/(sizeof (long))];

int x['\xBb' == (char) 187 ? 1: -1];


void func(int x)
{
  switch (x) {
    case sizeof("abc"): break;
    case sizeof("loooong"): func(4);
    case sizeof("\ploooong"): func(4);
  }
}

int expr;
char y[__builtin_constant_p(expr) ? -1 : 1];
char z[__builtin_constant_p(4) ? 1 : -1];


int comma1[0?1,2:3];
int comma2[1 || (1, 2)];


int comma3[(1, 2)];



char pbcp[__builtin_constant_p(4) ? (intptr_t)&expr : 0];

int illegaldiv1a[1 || 1/0];
int illegaldiv1b[1 && 1/0];

int illegaldiv2[1/0];
int illegaldiv3[(-2147483647 -1) / -1];

int illegaldiv4[0 / (1 / 0)];

int chooseexpr[__builtin_choose_expr(1, 1, expr)];
int realop[(__real__ 4) == 4 ? 1 : -1];
int imagop[(__imag__ 4) == 0 ? 1 : -1];

int *PR14729 = 0 ?: 1/0;

int bcp_call_v;
int bcp_call_a[] = {__builtin_constant_p(bcp_call_v && 0) ? bcp_call_v && 0 : -1};
