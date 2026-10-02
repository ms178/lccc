// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "C/C2y/n3469.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "C/C2y/n3469.c" 2








void test() {
  (void)_Countof(int[12]);
  (void)_Lengthof(int[12]);

}





# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdcountof.h" 1
# 20 "C/C2y/n3469.c" 2

