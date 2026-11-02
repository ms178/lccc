// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify -triple=i686-pc-linux-gnu %s

int a(void) {
  __builtin_va_arg((char*)0, int); // expected-error {{expression is not assignable}}
  __builtin_va_arg((void*){0}, int); // expected-error {{first argument to 'va_arg' is of type 'void *'}}
}
