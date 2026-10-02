// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c89: --c89: --c99: --c11: --c17: --c23
# 1 "C/drs/dr0xx.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 410 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/drs/dr0xx.c" 2
# 98 "C/drs/dr0xx.c"
int dr004(void) {return 0;}
int dr004(void) {return 1;}





struct dr007_a;
struct dr007_a;
struct dr007_a {int a;};
struct dr007_a;
struct dr007_b {int a;};
struct dr007_b;
# 119 "C/drs/dr0xx.c"
typedef int dr009_t;
void dr009_f((dr009_t));





typedef int dr010_t[];
dr010_t dr010_a = {1};
dr010_t dr010_b = {1, 2};
int dr010_c = sizeof(dr010_t);
# 142 "C/drs/dr0xx.c"
static int dr011_a[];


void dr011(void) {
  extern int i[];
  {

    extern int i[10];
    (void)sizeof(i);
    _Static_assert(sizeof(i) == 10 * sizeof(int), "fail");
  }
  (void)sizeof(i);

  extern int dr011_a[10];
  (void)sizeof(dr011_a);
  _Static_assert(sizeof(dr011_a) == 10 * sizeof(int), "fail");

  extern int j[10];
  {
    extern int j[];
    (void)sizeof(j);
    _Static_assert(sizeof(j) == 10 * sizeof(int), "fail");
  }
}




void dr012(void *p) {

  (void)&*p;

}




int dr013(int a[4]);
int dr013(int a[5]);
int dr013(int *a);

struct dr013_t {
struct dr013_t *p;
} dr013_v[sizeof(struct dr013_t)];




void dr015(void) {
  struct S {
    int small_int_bitfield : 16;
    unsigned int small_uint_bitfield : 16;
    int int_bitfield : 32;
    unsigned int uint_bitfield : 32;
  } s;
  _Static_assert(__builtin_types_compatible_p(__typeof__(+s.small_int_bitfield), int), "fail");
  _Static_assert(__builtin_types_compatible_p(__typeof__(+s.small_uint_bitfield), int), "fail");
  _Static_assert(__builtin_types_compatible_p(__typeof__(+s.int_bitfield), int), "fail");
  _Static_assert(__builtin_types_compatible_p(__typeof__(+s.uint_bitfield), unsigned int), "fail");
}






_Static_assert(((1) + (1)) == 2, "fail");







_Static_assert(__builtin_types_compatible_p(struct S { int a; }, union U { int a; }), "fail");




void dr031(int i) {
  switch (i) {
  case 2147483647 + 1: break;
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wswitch"



  case 2147483647 + 2ul: break;
#pragma clang diagnostic pop
  case (2147483647 * 4) / 4: break;
  }
}







int dr032 = (1, 2);





void dr035_1(a, b)
  int a(enum b {x, y});
  int b; {
  int test = x;
}

void dr035_2(c)
  enum m{q, r} c; {



  int test = q;
}
# 268 "C/drs/dr0xx.c"
_Static_assert(0x000E + 0x0100 == 0x000E + 0x0100, "fail");




_Static_assert(sizeof('a') == sizeof(int), "fail");






struct dr040 {
  char c;
  short s;
  int i[__builtin_offsetof(struct dr040, s)];
};




void dr043(void) {
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
# 128 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_offsetof.h" 1
# 129 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 291 "C/drs/dr0xx.c" 2






   (void)(void *)((void*)0);
# 306 "C/drs/dr0xx.c"
   ((void*)0)->a;
}




void dr044(void) {
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 1
# 88 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_ptrdiff_t.h" 1
# 89 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_size_t.h" 1
# 94 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 103 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_wchar_t.h" 1
# 104 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 128 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stddef_offsetof.h" 1
# 129 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stddef.h" 2
# 314 "C/drs/dr0xx.c" 2
 struct S { int a, b; };

  _Static_assert(__builtin_offsetof(struct S, b) == sizeof(int), "fail");
}




typedef int dr046_t;
int dr046(int dr046_t) { return dr046_t; }




struct dr047_t;
struct dr047_t *dr047_1(struct dr047_t *p) {return p; }
struct dr047_t *dr047_2(struct dr047_t a[]) {return a; }
int *dr047_3(int a2[][]) {return *a2; }
extern struct dr047_t es1;
extern struct dr047_t es2[1];




void dr050(void) {





  (void)L"huttah!";
  (void)NULL;
}






void dr053(void) {
  int f(int);
  int (*fp1)(int);
  int (*fp2)();
  int (**fpp)();

  fp1 = f;
  fp2 = fp1;
  (*fp2)(3);
  fpp = &fp1;
  (**fpp)(3);
}





char *dr064_1(int i, int *pi) {
  *pi = i;
  return 0;
}

char *dr064_2(int i, int *pi) {
  return (*pi = i, 0);
}




void dr068(void) {
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
# 384 "C/drs/dr0xx.c" 2



  _Static_assert('\xFF' == -1, "fail");




}
# 407 "C/drs/dr0xx.c"
void dr070_1(c)
  int c; {
}

void dr070_2(void) {
  dr070_1(6);
  dr070_1(6U);
}





enum dr071_t { foo_A = 0, foo_B = 1, foo_C = 8 };
void dr071(void) {



  _Static_assert(100 == (int)(enum dr071_t)100, "fail");
}




void dr081(void) {



 _Static_assert(-1 << 1 == -2, "fail");

 _Static_assert(1 << 3 == 1u << 3u, "fail");
}
# 447 "C/drs/dr0xx.c"
struct dr084_t;
extern void (*dr084_1)(struct dr084_t);
void dr084_2(struct dr084_t);
void dr084_2(struct dr084_t val) {}




struct dr088_t_1;

void dr088_f(struct dr088_t_1 *);
void dr088_1(void) {

  struct dr088_t_1;

  dr088_f((struct dr088_t_1 *)0);
}

void dr088_2(struct dr088_t_1 *p) { }
struct dr088_t_1 { int i; };
void dr088_3(struct dr088_t_1 s) {



  dr088_2(&s);
}
# 483 "C/drs/dr0xx.c"
void dr095(void) {



  struct One {
    int a;
  } one;
  struct Two {
    float f;
  } two = one;

  two = one;
}




void dr096(void) {
  typedef void func_type(void);
  func_type array_funcs[10];

  void array_void[10];

  struct S;
  struct S s[10];

  union U;
  union U u[10];
  union U { int i; };

  int never_completed_incomplete_array[][];

  extern int completed_later[][];
  extern int completed_later[10][10];
}




void dr098(void) {
  typedef void func_type(void);
  func_type fp;
  struct incomplete *incomplete_ptr;

  ++fp;
  fp++;
  --fp;
  fp--;

  (*incomplete_ptr)++;
  ++(*incomplete_ptr);
  (*incomplete_ptr)--;
  --(*incomplete_ptr);
}
