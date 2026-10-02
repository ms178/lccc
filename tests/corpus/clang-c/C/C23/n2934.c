// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23: --c17
# 1 "C/C23/n2934.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 444 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/C23/n2934.c" 2







thread_local struct S {


  bool b;

} s;

static_assert(alignof(int) != 0, "");





# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdalign.h" 1
# 22 "C/C23/n2934.c" 2
# 41 "C/C23/n2934.c"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdbool.h" 1
# 42 "C/C23/n2934.c" 2
# 59 "C/C23/n2934.c"
struct GH81472 {
  char alignas(8) a1;
  alignas(8) char a2;
  char _Alignas(8) a3;
  _Alignas(8) char a4;
  char a5 alignas(8);
  char a6 _Alignas(8);
};




struct alignas(8) Reject1 {

  int a;
};

struct _Alignas(8) Reject2 {

  int a;
};
