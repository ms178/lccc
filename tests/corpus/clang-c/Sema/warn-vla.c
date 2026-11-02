// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99: --c89
// RUN: %clang_cc1 -std=c99 -fsyntax-only -verify -Wvla %s
// RUN: %clang_cc1 -std=c89 -fsyntax-only -verify -Wvla %s

void test1(int n) {
  int v[n]; // expected-warning {{variable length array}}
}

void test2(int n, int v[n]) { // expected-warning {{variable length array}}
}

void test3(int n, int v[n]); // expected-warning {{variable length array}}

