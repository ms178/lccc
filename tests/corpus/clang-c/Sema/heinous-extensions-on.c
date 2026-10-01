// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -verify -Wno-error=invalid-gnu-asm-cast

void foo(void) {
  int a;
  // PR3788
  asm("nop" : : "m"((int)(a))); // expected-warning {{invalid use of a cast in an inline asm context requiring an lvalue}}
  // PR3794
  asm("nop" : "=r"((unsigned)a)); // expected-warning {{invalid use of a cast in an inline asm context requiring an lvalue}}
}
