// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/init.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/init.c" 2


# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 1
# 84 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_header_macro.h" 1
# 85 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2



# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_ptrdiff_t.h" 1
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_ptrdiff_t.h"
typedef long int ptrdiff_t;
# 89 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h" 1
# 18 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h"
typedef long unsigned int size_t;
# 94 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 103 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_wchar_t.h" 1
# 24 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_wchar_t.h"
typedef int wchar_t;
# 104 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_null.h" 1
# 109 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 123 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_max_align_t.h" 1
# 19 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_max_align_t.h"
typedef struct {
  long long __clang_max_align_nonce1
      __attribute__((__aligned__(__alignof__(long long))));
  long double __clang_max_align_nonce2
      __attribute__((__aligned__(__alignof__(long double))));
} max_align_t;
# 124 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_offsetof.h" 1
# 129 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 4 "Sema/init.c" 2
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
# 5 "Sema/init.c" 2

typedef void (* fp)(void);
void foo(void);


fp a[(short int)1] = { foo };

int myArray[5] = {1, 2, 3, 4, 5};
int *myPointer2 = myArray;
int *myPointer = &(myArray[2]);


extern int x;
void *g = &x;
int *h = &x;

struct union_crash
{
    union
    {
    };
};

int test(void) {
  int a[10];
  int b[10] = a;
  int +;

  struct union_crash u = { .d = 1 };
}



struct cdiff_cmd {
          const char *name;
          unsigned short argc;
          int (*handler)(void);
};
int cdiff_cmd_open(void);
struct cdiff_cmd commands[] = {
        {"OPEN", 1, &cdiff_cmd_open }
};


static struct { int z; } s[2];
int *t = &(*s).z;


short *a2(void)
{
  short int b;
  static short *bp = &b;

  return bp;
}

int pbool(void) {
  typedef const _Bool cbool;
  _Bool pbool1 = (void *) 0;
  cbool pbool2 = &pbool;
  return pbool2;
}


union { float f; unsigned u; } u = { 1.0f };

int f3(int x) { return x; }
typedef void (*vfunc)(void);
void *bar = (vfunc) f3;


struct sym_reg {
        char nc_gpreg;
};
int sym_fw1a_scr[] = {
           ((int)(&((struct sym_reg *)0)->nc_gpreg)) & 0,
           8 * ((int)(&((struct sym_reg *)0)->nc_gpreg))
};


struct s1 s2 = {

    .a = sizeof(struct s3),

    .b = bogus
}


char t[] = ("Hello");

typedef struct { } empty;

typedef struct {
  empty e;
  int i2;
} st;

st st1 = { .i2 = 1 };

struct {
  int a;
  int z[2];
} y = { .z = {} };

int bbb[10];

struct foo2 {
   uintptr_t a;
};

struct foo2 bar2[] = {
   { (intptr_t)bbb }
};

struct foo2 bar3 = { 1, 2 };
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wexcess-initializers"
struct foo2 bar3_silent = {1, 2};
#pragma clang diagnostic pop

int* ptest1 = __builtin_choose_expr(1, (int*)0, (int*)0);

typedef int32_t ivector4 __attribute((vector_size(16)));
ivector4 vtest1 = 1 ? (ivector4){1} : (ivector4){1};
ivector4 vtest2 = __builtin_choose_expr(1, (ivector4){1}, (ivector4){1});

uintptr_t ptrasintadd1 = (uintptr_t)&a - 4;
uintptr_t ptrasintadd2 = (uintptr_t)&a + 4;
uintptr_t ptrasintadd3 = 4 + (uintptr_t)&a;


const wchar_t widestr[] = L"asdf";


const double pr5447 = (0.05 < -1.0) ? -1.0 : 0.0499878;







int PR4386_bar(void);
int PR4386_foo(void) __attribute((weak));
int PR4386_zed(void);

int PR4386_a = ((void *) PR4386_bar) != 0;
int PR4386_b = ((void *) PR4386_foo) != 0;
int PR4386_c = ((void *) PR4386_zed) != 0;
int PR4386_zed(void) __attribute((weak));


typedef char strty[10];
struct vortexstruct { strty s; };
struct vortexstruct vortexvar = { "asdf" };

typedef struct { uintptr_t x : 2; } StructWithBitfield;
StructWithBitfield bitfieldvar = { (uintptr_t)&bitfieldvar };


struct PR4517_foo {
  int x;
};
struct PR4517_bar {
  struct PR4517_foo foo;
};
const struct PR4517_foo my_foo = {.x = 42};
struct PR4517_bar my_bar = {
    .foo = my_foo,
};
struct PR4517_bar my_bar2 = (struct PR4517_bar){
    .foo = my_foo,
};
struct PR4517_bar my_bar3 = {
    my_foo,
};
struct PR4517_bar my_bar4 = (struct PR4517_bar){
    my_foo
};
extern const struct PR4517_foo my_foo2;
struct PR4517_bar my_bar5 = {
  .foo = my_foo2,
};
const struct PR4517_foo my_foo3 = {.x = my_foo.x};
int PR4517_a[2] = {0, 1};
const int PR4517_ca[2] = {0, 1};
int PR4517_idx = 0;
const int PR4517_idxc = 1;
int PR4517_x1 = PR4517_a[PR4517_idx];
int PR4517_x2 = PR4517_a[PR4517_idxc];
int PR4517_x3 = PR4517_a[0];
int PR4517_y1 = PR4517_ca[PR4517_idx];
int PR4517_y2 = PR4517_ca[PR4517_idxc];
int PR4517_y3 = PR4517_ca[0];
union PR4517_u {
    int x;
    float y;
};
const union PR4517_u u1 = {4.0f};
const union PR4517_u u2 = u1;
const union PR4517_u u3 = {u1.y};
