// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
struct foo {
  int a;
};

int main(void) {
  struct foo xxx;
  int i;

  xxx = (struct foo)1;  // expected-error {{used type 'struct foo' where arithmetic or pointer type is required}}
  i = (int)xxx; // expected-error {{operand of type 'struct foo' where arithmetic or pointer type is required}}
}
