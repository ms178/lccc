// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
# 1 "Sema/nullptr.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 444 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/nullptr.c" 2

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
# 3 "Sema/nullptr.c" 2

typedef typeof(nullptr) nullptr_t;

struct A {};

__attribute__((overloadable)) int o1(char*);
__attribute__((overloadable)) void o1(uintptr_t);

nullptr_t f(nullptr_t null)
{

  null = nullptr;
  void *p = nullptr;
  p = null;
  int *pi = nullptr;
  pi = null;
  null = 0;
  bool b = nullptr;


  uintptr_t i = nullptr;


  (void)(null == nullptr);
  (void)(null <= nullptr);
  (void)(null == 0);
  (void)(null == (void*)0);
  (void)((void*)0 == nullptr);
  (void)(null <= 0);
  (void)(null <= (void*)0);
  (void)((void*)0 <= nullptr);
  (void)(0 == nullptr);
  (void)(nullptr == 0);
  (void)(nullptr <= 0);
  (void)(0 <= nullptr);
  (void)(1 > nullptr);
  (void)(1 != nullptr);
  (void)(1 + nullptr);
  (void)(0 ? nullptr : 0);
  (void)(0 ? nullptr : (void*)0);
  (void)(0 ? nullptr : (struct A){});
  (void)(0 ? (struct A){} : nullptr);


  int t = o1(nullptr);
  t = o1(null);


  (void)&nullptr;
  nullptr_t *pn = &null;

  int *ip = *pn;
  if (*pn) { }
}

__attribute__((overloadable)) void *g(void*);
__attribute__((overloadable)) bool g(bool);


static_assert(__builtin_types_compatible_p(typeof(g(nullptr)), void *), "");

void sent(int, ...) __attribute__((sentinel));

void g() {

  sent(10, nullptr);
}

void printf(const char*, ...) __attribute__((format(printf, 1, 2)));

void h() {

  printf("%p", nullptr);
}

static_assert(sizeof(nullptr_t) == sizeof(void*), "");

static_assert(!nullptr, "");
static_assert(!(bool){nullptr}, "");

static_assert(!(nullptr < nullptr), "");
static_assert(!(nullptr > nullptr), "");
static_assert( nullptr <= nullptr, "");
static_assert( nullptr >= nullptr, "");
static_assert( nullptr == nullptr, "");
static_assert(!(nullptr != nullptr), "");

static_assert(!(0 < nullptr), "");
static_assert(!(0 > nullptr), "");
static_assert( 0 <= nullptr, "");
static_assert( 0 >= nullptr, "");
static_assert( 0 == nullptr, "");
static_assert(!(0 != nullptr), "");

static_assert(!(nullptr < 0), "");
static_assert(!(nullptr > 0), "");
static_assert( nullptr <= 0, "");
static_assert( nullptr >= 0, "");
static_assert( nullptr == 0, "");
static_assert(!(nullptr != 0), "");

__attribute__((overloadable)) int f1(int*);
__attribute__((overloadable)) float f1(bool);

void test_f1() {
  int ir = (f1)(nullptr);
}


void foo(void *);
void bar() { foo(__nullptr); }
static_assert(nullptr == __nullptr);
static_assert(__nullptr == 0);
static_assert(_Generic(typeof(__nullptr), nullptr_t: true, default: false));
static_assert(_Generic(__typeof(__nullptr), int : 0, void * : 0, default : 1));
