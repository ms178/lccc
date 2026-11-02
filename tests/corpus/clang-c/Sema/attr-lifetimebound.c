// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99
// RUN: %clang_cc1 -std=c99 -verify %s

int *f(int* p __attribute__((lifetimebound)));

int *g() {
  int i;
  return f(&i); // expected-warning {{address of stack memory associated with local variable 'i' returned}}
}
