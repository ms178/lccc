// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

// PR9137
void f0(int x) : {}; // expected-error{{expected function body after function declarator}}
void f1(int x) try {}; // expected-error{{expected function body after function declarator}}

// GH73559
::; // expected-error{{expected identifier or '('}}
