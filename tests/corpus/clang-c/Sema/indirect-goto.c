// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

struct c {int x;};
int a(struct c x, long long y) {
  void const* l1_ptr = &&l1;
  goto *l1_ptr;
l1:
  goto *x; // expected-error{{incompatible type}}
  goto *y; // expected-error{{incompatible integer to pointer conversion}}
}

