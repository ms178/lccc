// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --ms_extensions --c
// RUN: %clang_cc1 %s -fsyntax-only -Wmicrosoft -verify -fms-extensions

void f(void) {
  int a;

  __try a; // expected-error {{expected '{'}} expected-warning {{expression result unused}}

  __try {
  }
} // expected-error {{expected '__except' or '__finally' block}}

void g(void) {
  int a;

  __try {
  } __except(1) a; // expected-error {{expected '{'}} expected-warning {{expression result unused}}
}

void h(void) {
  int a;

  __try {
  } __finally a; // expected-error {{expected '{'}} expected-warning {{expression result unused}}
}
