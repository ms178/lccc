// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify -pedantic %s
_Complex double X;
void test1(int c) {
  X = 5;
}
void test2(void) {
  int i;
  double d = i;
  double _Complex a = 5;

  test1(a);
  a = 5;
  d = i;
}
int test3(void) {
  int a[2];
  a[0] = test3; // expected-error{{incompatible pointer to integer conversion assigning to 'int' from 'int (void)'}}
  return 0;
}
short x; void test4(char c) { x += c; }
int y; void test5(char c) { y += c; }
