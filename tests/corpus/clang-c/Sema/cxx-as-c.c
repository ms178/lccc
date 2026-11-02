// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify

// PR36157
struct Foo {
  Foo(int n) : n_(n) {} // expected-error 1+{{}}
private:
  int n;
};
int main() { Foo f; } // expected-error 1+{{}}
