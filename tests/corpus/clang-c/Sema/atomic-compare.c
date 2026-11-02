// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -verify -fsyntax-only -Wno-logical-not-parentheses

void f(_Atomic(int) a, _Atomic(int) b) {
  if (a > b)      {} // no warning
  if (a < b)      {} // no warning
  if (a >= b)     {} // no warning
  if (a <= b)     {} // no warning
  if (a == b)     {} // no warning
  if (a != b)     {} // no warning

  if (a == 0) {} // no warning
  if (a > 0) {} // no warning
  if (a > 1) {} // no warning
  if (a > 2) {} // no warning

  if (!a > 0) {}  // no warning
  if (!a > 1)     {} // expected-warning {{comparison of constant 1 with boolean expression is always false}}
  if (!a > 2)     {} // expected-warning {{comparison of constant 2 with boolean expression is always false}}
  if (!a > b)     {} // no warning
  if (!a > -1)    {} // expected-warning {{comparison of constant -1 with boolean expression is always true}}
}

typedef _Atomic(int) Ty;
void PR23638(Ty *a) {
  if (*a == 1) {} // no warning
}
