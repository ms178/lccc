// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
/* RUN: %clang_cc1 %s -ast-print
 */

void foo() {
  int X;
  X = sizeof(void (*(*)())());
  X = sizeof(int(*)(int, float, ...));
  X = sizeof(void (*(int arga, void (*argb)(double Y)))(void* Z));
}

