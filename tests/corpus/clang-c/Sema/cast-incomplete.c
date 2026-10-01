// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s -verify
// PR5692

enum x;            // expected-note   {{forward declaration}}
extern struct y a; // expected-note   {{forward declaration}}
extern union z b;  // expected-note 2 {{forward declaration}}

void foo(void) {
  (enum x)1;   // expected-error {{cast to incomplete type}}
  (struct y)a; // expected-error {{cast to incomplete type}}
  (union z)b;  // expected-error {{cast to incomplete type}}
  (union z)1;  // expected-error {{cast to incomplete type}}
}

