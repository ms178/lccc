// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -verify %s

void test1(void)
{
  #pragma clang __debug captured x // expected-warning {{extra tokens at end of #pragma clang __debug captured directive}}
  {
  }
}

void test2(void)
{
  #pragma clang __debug captured
  int x; // expected-error {{expected '{'}}
}
