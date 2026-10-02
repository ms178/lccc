// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
# 1 "C/C23/n3030.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 444 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/C23/n3030.c" 2


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
# 4 "C/C23/n3030.c" 2

enum us : unsigned short {
  us_max = (32767 * 2 + 1),
  us_violation,
  us_violation_2 = us_max + 1,
  us_wrap_around_to_zero = (unsigned short)((32767 * 2 + 1) + 1)


};

enum ui : unsigned int {
  ui_max = (2147483647 *2U +1U),
  ui_violation,
  ui_no_violation = ui_max + 1,
  ui_wrap_around_to_zero = (unsigned int)((2147483647 *2U +1U) + 1)
};

enum E1 : short;
enum E2 : short;
enum E3;
enum E4 : unsigned long long;

enum E1 : short { m11, m12 };
enum E1 x = m11;

enum E2 : long {
  m21,
  m22
};

enum E3 {

  m31,
  m32,
  m33 = sizeof(enum E3)
};
enum E3 : int;

enum E4 : unsigned long long {
  m40 = sizeof(enum E4),
  m41 = (9223372036854775807LL*2ULL+1ULL),
  m42
};

enum E5 y;


enum E6 : long int z;
enum E7 : long int = 0;


enum underlying : unsigned char { b0 };

constexpr int a = _Generic(b0, int: 2, unsigned char: 1, default: 0);
constexpr int b = _Generic((enum underlying)b0, int: 2, unsigned char: 1, default: 0);
static_assert(a == 1);
static_assert(b == 1);

void f1(enum a : long b);

void f2(enum c : long{x} d);
enum e : int f3();

typedef enum t u;
typedef enum v : short W;
typedef enum q : short { s } R;

struct s1 {
  int x;
  enum e:int : 1;
  int y;
};

enum forward;
extern enum forward fwd_val0;
extern enum forward *fwd_ptr0;
extern int
    *fwd_ptr0;

enum forward1 : int;
extern enum forward1 fwd_val1;
extern int fwd_val1;
extern enum forward1 *fwd_ptr1;
extern int *fwd_ptr1;

enum ee1 : short;
enum e : short f = 0;
enum g : short { yyy } h = yyy;

enum ee2 : typeof ((enum ee3 : short { A })0, (short)0);

enum not_actually_atomic : _Atomic(short) {
  Surprise
};

enum not_actually_const : const int {
  SurpriseAgain
};

enum not_actually_volatile : volatile int {
  SurpriseOnceMore
};

enum not_acually_const_or_volatile : const volatile int {
  WhyTheSurprise
};
