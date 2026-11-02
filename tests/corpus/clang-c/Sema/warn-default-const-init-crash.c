// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

// This invalid code was causing a stack overflow, check that we issue
// reasonable diagnostics and not crash.
struct GH140887 {    // expected-note {{definition of 'struct GH140887' is not complete until the closing '}'}}
  struct GH140887 s; // expected-error {{field has incomplete type 'struct GH140887'}}
};

void gh140887() {
  struct GH140887 s;
}
