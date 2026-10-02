// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s -verify -fblocks

void foo(void) {
  ^ (void) __attribute__((noreturn)) { }(); // expected-error {{block declared 'noreturn' should not return}}
}
