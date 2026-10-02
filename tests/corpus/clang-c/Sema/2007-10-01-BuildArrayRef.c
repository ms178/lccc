// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
// PR 1603
void func(void)
{
   const int *arr;
   arr[0] = 1;  // expected-error {{read-only variable is not assignable}}
}

struct foo {
  int bar;
};
struct foo sfoo = { 0 };

int func2(void)
{
  const struct foo *fp;
  fp = &sfoo;
  fp[0].bar = 1;  // expected-error {{read-only variable is not assignable}}
  return sfoo.bar;
}
