// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "Sema/pragma-pack-2.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/pragma-pack-2.c" 2



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
# 5 "Sema/pragma-pack-2.c" 2

#pragma pack(4)


struct s0 {
  char f0;
  int f1;
};
extern int a0[__builtin_offsetof(struct s0, f1) == 4 ? 1 : -1];

#pragma pack(push, 2)
struct s1 {
  char f0;
  int f1;
};
extern int a1[__builtin_offsetof(struct s1, f1) == 2 ? 1 : -1];
#pragma pack(pop)

#pragma pack(1)
struct s3_0 {
  char f0;
  int f1;
};
#pragma pack()
struct s3_1 {
  char f0;
  int f1;
};
extern int a3_0[__builtin_offsetof(struct s3_0, f1) == 1 ? 1 : -1];
extern int a3_1[__builtin_offsetof(struct s3_1, f1) == 4 ? 1 : -1];


#pragma pack(1)
struct s4_0 {
  char f0;
  int f1;
};
#pragma pack(0)
struct s4_1 {
  char f0;
  int f1;
};
extern int a4_0[__builtin_offsetof(struct s4_0, f1) == 1 ? 1 : -1];
extern int a4_1[__builtin_offsetof(struct s4_1, f1) == 4 ? 1 : -1];

void f(void) {
#pragma pack(push, 2)
  struct s5_0 {
    char f0;
    struct s2_4_0 {
      int f0;
    } f1;
  };
#pragma pack(pop)
  extern int s5_0[__builtin_offsetof(struct s5_0, f1) == 2 ? 1 : -1];
}
