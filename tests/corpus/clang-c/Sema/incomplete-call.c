// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wno-strict-prototypes -verify %s

struct foo; // expected-note 3 {{forward declaration of 'struct foo'}}

struct foo a(void); // expected-note {{'a' declared here}}
void b(struct foo);
void c();

void func(void *p) {
  a(); // expected-error{{calling 'a' with incomplete return type 'struct foo'}}
  b(*(struct foo*)p); // expected-error{{argument type 'struct foo' is incomplete}}
  c(*(struct foo*)p); // expected-error{{argument type 'struct foo' is incomplete}}
}
