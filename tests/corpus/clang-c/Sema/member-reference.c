// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify -fsyntax-only

struct simple { int i; };

void f(void) {
   struct simple s[1];
   s->i = 1;
}

typedef int x;
struct S {
  int x;
  x z;
};

void g(void) {
  struct S s[1];
  s->x = 1;
  s->z = 2;
}

int PR17762(struct simple c) {
  return c->i; // expected-error {{member reference type 'struct simple' is not a pointer; did you mean to use '.'?}}
}
