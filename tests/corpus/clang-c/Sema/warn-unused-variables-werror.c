// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wunused-variable -Werror -verify %s

void f(void) {
  int i;  // expected-error{{unused}}
  int j;  // expected-error{{unused}}
}
