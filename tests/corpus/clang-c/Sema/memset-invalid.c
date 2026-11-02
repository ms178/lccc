// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s -verify

char memset(void); // expected-warning {{incompatible redeclaration of library function 'memset'}} expected-note{{'memset' is a builtin with type}}
char test(void) {
  return memset();
}
