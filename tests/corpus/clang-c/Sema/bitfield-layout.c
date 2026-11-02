// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "Sema/bitfield-layout.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/bitfield-layout.c" 2








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
# 10 "Sema/bitfield-layout.c" 2
# 19 "Sema/bitfield-layout.c"
struct a {char x; int : 0; char y;};




extern int a_1[sizeof(struct a) == 5 ? 1 : -1];
extern int a_2[__alignof(struct a) == 1 ? 1 : -1];



struct __attribute__((packed)) a2 {
  short x : 9;
  char : 0;


  int y : 17;


};

extern int a2_1[sizeof(struct a2) == 5 ? 1 : -1];
extern int a2_2[__alignof(struct a2) == 1 ? 1 : -1];


struct __attribute__((packed)) a3 {
  short x : 9;
  int : 0;


};





extern int a3_1[sizeof(struct a3) == 4 ? 1 : -1];
extern int a3_2[__alignof(struct a3) == 1 ? 1 : -1];



struct __attribute__((packed)) a4 {
  short x : 9;
  int : 1;


};
extern int a4_1[sizeof(struct a4) == 2 ? 1 : -1];
extern int a4_2[__alignof(struct a4) == 1 ? 1 : -1];

union b {char x; int : 0; char y;};




extern int b_1[sizeof(union b) == 1 ? 1 : -1];
extern int b_2[__alignof(union b) == 1 ? 1 : -1];



struct c {char x; int : 20;};




extern int c_1[sizeof(struct c) == 4 ? 1 : -1];
extern int c_2[__alignof(struct c) == 1 ? 1 : -1];


union d {char x; int : 20;};




extern int d_1[sizeof(union d) == 3 ? 1 : -1];
extern int d_2[__alignof(union d) == 1 ? 1 : -1];



struct __attribute__((packed)) e {int x : 4, y : 30, z : 30;};
extern int e_1[sizeof(struct e) == 8 ? 1 : -1];
extern int e_2[__alignof(struct e) == 1 ? 1 : -1];


struct f {__attribute((aligned(8))) int x : 30, y : 30, z : 30;};
extern int f_1[sizeof(struct f) == 24 ? 1 : -1];
extern int f_2[__alignof(struct f) == 8 ? 1 : -1];


struct s0 {
  char a[0x32100000];
  int x:30, y:30;
};

extern int s0_1[sizeof(struct s0) == 0x32100008 ? 1 : -1];
extern int s0_2[__alignof(struct s0) == 4 ? 1 : -1];


struct g0 {
  char a;
  __attribute__((aligned(16))) int b : 1;
  char c;
};






extern int g0_1[sizeof(struct g0) == 32 ? 1 : -1];;
extern int g0_2[__alignof(struct g0) == 16 ? 1 : -1];;
extern int g0_3[__builtin_offsetof(struct g0, c) == 17 ? 1 : -1];;



struct g1 {
  char a;
  __attribute__((aligned(2))) int b : 1;
  char c;
};

extern int g1_1[sizeof(struct g1) == 4 ? 1 : -1];;
extern int g1_2[__alignof(struct g1) == 4 ? 1 : -1];;



extern int g1_3[__builtin_offsetof(struct g1, c) == 3 ? 1 : -1];;



struct g2 {
  char a;
  int b : 1;
  char c;
};

extern int g2_1[sizeof(struct g2) == 4 ? 1 : -1];;
extern int g2_2[__alignof(struct g2) == 4 ? 1 : -1];;
extern int g2_3[__builtin_offsetof(struct g2, c) == 2 ? 1 : -1];;



struct __attribute__((packed)) g3 {
  char a;
  __attribute__((aligned(16))) int b : 1;
  char c;
};

extern int g3_2[__alignof(struct g3) == 16 ? 1 : -1];;




extern int g3_1[sizeof(struct g3) == 32 ? 1 : -1];;
extern int g3_3[__builtin_offsetof(struct g3, c) == 17 ? 1 : -1];;


struct __attribute__((packed)) g4 {
  char a;
  __attribute__((aligned(2))) int b : 1;
  char c;
};

extern int g4_1[sizeof(struct g4) == 4 ? 1 : -1];;
extern int g4_2[__alignof(struct g4) == 2 ? 1 : -1];;



extern int g4_3[__builtin_offsetof(struct g4, c) == 3 ? 1 : -1];;


struct g5 {
  char : 1;
  __attribute__((aligned(1))) int n : 24;


};
extern int g5_1[sizeof(struct g5) == 4 ? 1 : -1];;
extern int g5_2[__alignof(struct g5) == 4 ? 1 : -1];;

struct __attribute__((packed)) g6 {
  char : 1;
  __attribute__((aligned(1))) int n : 24;


};
extern int g6_1[sizeof(struct g6) == 4 ? 1 : -1];;
extern int g6_2[__alignof(struct g6) == 1 ? 1 : -1];;

struct g7 {
  char : 1;
  __attribute__((aligned(1))) int n : 25;


};



extern int g7_1[sizeof(struct g7) == 8 ? 1 : -1];;

extern int g7_2[__alignof(struct g7) == 4 ? 1 : -1];;

struct __attribute__((packed)) g8 {
  char : 1;
  __attribute__((aligned(1))) int n : 25;


};



extern int g8_1[sizeof(struct g8) == 5 ? 1 : -1];;

extern int g8_2[__alignof(struct g8) == 1 ? 1 : -1];;

struct g9 {
  __attribute__((aligned(1))) char a : 2, b : 2, c : 2, d : 2, e : 2;
  int i;
};



extern int g9_1[sizeof(struct g9) == 12 ? 1 : -1];;

extern int g9_2[__alignof(struct g9) == 4 ? 1 : -1];;

struct __attribute__((packed)) g10 {
  __attribute__((aligned(1))) char a : 2, b : 2, c : 2, d : 2, e : 2;
  int i;
};



extern int g10_1[sizeof(struct g10) == 9 ? 1 : -1];;

extern int g10_2[__alignof(struct g10) == 1 ? 1 : -1];;

struct g11 {
  char a;
  __attribute__((aligned(1))) long long b : 62;
  char c;
};

extern int g11_1[sizeof(struct g11) == 24 ? 1 : -1];;
extern int g11_2[__alignof(struct g11) == 8 ? 1 : -1];;
extern int g11_3[__builtin_offsetof(struct g11, c) == 16 ? 1 : -1];;






struct __attribute__((packed)) g12 {
  char a;
  __attribute__((aligned(1))) long long b : 62;
  char c;
};
extern int g12_1[sizeof(struct g12) == 10 ? 1 : -1];;
extern int g12_2[__alignof(struct g12) == 1 ? 1 : -1];;
extern int g12_3[__builtin_offsetof(struct g12, c) == 9 ? 1 : -1];;

struct g13 {
  char a;
  __attribute__((aligned(1))) long long : 0;
  char c;
};





extern int g13_1[sizeof(struct g13) == 9 ? 1 : -1];;
extern int g13_2[__alignof(struct g13) == 1 ? 1 : -1];;
extern int g13_3[__builtin_offsetof(struct g13, c) == 8 ? 1 : -1];;






struct __attribute__((packed)) g14 {
  char a;
  __attribute__((aligned(1))) long long : 0;
  char c;
};





extern int g14_1[sizeof(struct g14) == 9 ? 1 : -1];;
extern int g14_2[__alignof(struct g14) == 1 ? 1 : -1];;
extern int g14_3[__builtin_offsetof(struct g14, c) == 8 ? 1 : -1];;
