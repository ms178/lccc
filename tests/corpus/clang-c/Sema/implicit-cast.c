// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

static char *test1(int cf) {
  return cf ? "abc" : 0;
}
static char *test2(int cf) {
  return cf ? 0 : "abc";
}

int baz(void) {
  int f;
  return ((void)0, f = 1.4f); // expected-warning {{implicit conversion from 'float' to 'int' changes value from 1.4 to 1}}
}
