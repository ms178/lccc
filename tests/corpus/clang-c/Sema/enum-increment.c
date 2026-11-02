// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s -verify
// expected-no-diagnostics
enum A { A1, A2, A3 };
typedef enum A A;
void test(void) {
  A a;
  a++;
  a--;
  ++a;
  --a;
  a = a + 1;
  a = a - 1;
}
