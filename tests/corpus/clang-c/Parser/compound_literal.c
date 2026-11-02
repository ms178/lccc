// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
// RUN: %clang_cc1 -fsyntax-only -verify -x c++ -Wno-dangling-assignment %s
// expected-no-diagnostics
int main(void) {
  char *s;
  // In C++ mode, the cast creates a "char [4]" array temporary here.
  s = (char []){"whatever"};  // dangling!
  s = (char(*)){s};
}
