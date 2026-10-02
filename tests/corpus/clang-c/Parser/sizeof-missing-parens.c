// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

void Foo(int);

#define Bar(x) Foo(x)

void Baz(void) {
  Foo(sizeof int); // expected-error {{expected parentheses around type name in sizeof expression}}
  Bar(sizeof int); // expected-error {{expected parentheses around type name in sizeof expression}}
}
