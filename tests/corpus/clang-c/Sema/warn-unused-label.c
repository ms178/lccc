// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wunused-label -verify %s

void f(void) {
  a:
  goto a;
  b: // expected-warning{{unused}}
  c: __attribute__((unused));
  d: __attribute__((noreturn)); // expected-warning {{'noreturn' attribute only applies to functions}}
  goto d;
  return;
}

void PR8455(void) {
  L: __attribute__((unused)) return; // ok, no semicolon required
}
