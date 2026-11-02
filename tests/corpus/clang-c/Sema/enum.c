// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
# 1 "Sema/enum.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 444 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/enum.c" 2


enum e {A,
        B = 42LL << 32,
      C = -4, D = 12456 };

enum f { a = -2147483648, b = 2147483647 };

enum g {
   c = -2147483649,
   d = 2147483647 };
enum h { e = -2147483648,
   f = 2147483648,
  i = 0xFFFF0000
};


enum x
{ y = -9223372036854775807LL-1,
z = 9223372036854775808ULL };

int test(void) {
  return sizeof(enum e) ;
}

enum gccForwardEnumExtension ve;



int test2(int i)
{
  ve + i;
}


union u0;
enum u0 { U0A };

extern enum some_undefined_enum ve2;

void test4(void) {
  for (; ve2;)
    ;
  (_Bool)ve2;

  for (; ;ve2)
    ;
  (void)ve2;
  ve2;
}


enum someenum {};

enum e0 {
  E0 = sizeof(enum e0 { E1 }),
};


enum { PR3173A, PR3173B = PR3173A+50 };


void foo(void) {
  enum xpto;
  enum xpto;
}

typedef enum { X = 0 };


enum NotYetComplete {
  NYC1 = sizeof(enum NotYetComplete)
};


struct s1 {
  enum e1 (*bar)(void);
};

enum e1 { YES, NO };

static enum e1 badfunc(struct s1 *q) {
  return q->bar();
}



typedef enum {
  an_enumerator = 20
} an_enum;
char * s = (an_enum) an_enumerator;


enum PR4515 {PR4515a=1u,PR4515b=(PR4515a-2)/2};
int CheckPR4515[PR4515b==0?1:-1];


extern enum PR7911T PR7911V;
void PR7911F(void) {
  switch (PR7911V)
    ;
}

char test5[1 ? 1 : -1];


void PR8694(int* e)
{
}

void crash(enum E *e)

{
        PR8694(e);
}

typedef enum { NegativeShort = (short)-1 } NegativeShortEnum;
int NegativeShortTest[NegativeShort == -1 ? 1 : -1];


enum Color { Red, Green, Blue };
typedef struct Color NewColor;




  static_assert(1);
  static_assert(1);
# 137 "Sema/enum.c"
typedef enum : unsigned char { Pink, Black, Cyan } Color;



struct PR28903 {
  enum {
    PR28903_A = (enum {
      PR28903_B,
      PR28903_C = PR28903_B
    })0
  };
  int makeStructNonEmpty;
};

static int EnumRedecl;
struct S {
  enum {
    EnumRedecl = 4
  } e;
};

union U {
  enum {
    EnumRedecl = 5
  } e;
};

enum PR15071 {
  PR15071_One
};

struct EnumRedeclStruct {
  enum {
    PR15071_One
  } e;
};

enum struct GH42372_1 {
  One
};






enum class GH42372_2 {
  One
};

enum IncOverflow {
  V2 = 2147483647,
  V3
};




enum GH59352 {
 BigVal = 66666666666666666666wb
};
_Static_assert(BigVal == 66666666666666666666wb);


_Static_assert(
    _Generic(BigVal,
    _BitInt(67) : 0,
    long int : 0,
    long unsigned int : 0,
    long long : 0,
    unsigned long long : 0,
    __int128_t : 0,
    __uint128_t : 1
    )
);

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
# 214 "Sema/enum.c" 2

void fooinc23() {
  enum E1 {
    V1 = 2147483647
  } e1;

  enum E2 {
    V2 = 2147483647,
    V3
  } e2;

  enum E3 {
    V4 = 2147483647,
    V5 = (-9223372036854775807L -1L)
  } e3;

  enum E4 {
    V6 = 1u,
    V7 = 2wb
  } e4;

  _Static_assert(_Generic(V1, int : 1));
  _Static_assert(_Generic(V2, int : 0, unsigned int : 1));
  _Static_assert(_Generic(V3, int : 0, unsigned int : 1));
  _Static_assert(_Generic(V4, int : 0, signed long : 1));
  _Static_assert(_Generic(V5, int : 0, signed long : 1));
  _Static_assert(_Generic(V6, int : 1));
  _Static_assert(_Generic(V7, int : 1));
  _Static_assert(_Generic((enum E4){}, unsigned int : 1));

}
