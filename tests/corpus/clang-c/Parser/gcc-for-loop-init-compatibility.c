// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c89: --c89 --strict_gnu: --c99 -DC99
// RUN: %clang_cc1 -std=c89 -fsyntax-only -verify %s
// RUN: %clang_cc1 -std=gnu89 -fsyntax-only -verify %s
// RUN: %clang_cc1 -std=c99 -fsyntax-only -verify %s -DC99

#ifdef C99
// expected-no-diagnostics
#endif

void foo(void) {
#ifndef C99
  // expected-warning@+2{{GCC does not allow variable declarations in for loop initializers before C99}}
#endif
  for (int i = 0; i < 10; i++)
    ;
}
