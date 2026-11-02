// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple arm-unknown-gnu -fsyntax-only -verify %s

void f(void) {
  struct EmptyStruct {};
  struct EmptyStruct S;
  __builtin_va_end(S); // no-crash, expected-error {{non-const lvalue reference to type '__builtin_va_list' cannot bind to a value of unrelated type 'struct EmptyStruct'}}
}