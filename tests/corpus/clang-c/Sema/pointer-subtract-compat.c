// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify -pedantic

typedef const char rchar;
int a(char* a, rchar* b) {
  return a-b;
}

void f0(void (*fp)(void)) {
  int x = fp - fp; // expected-warning{{arithmetic on pointers to the function type 'void (void)' is a GNU extension}}
}
