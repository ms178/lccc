// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c11
// RUN: %clang_cc1 -std=c11 -verify %s

struct GH31295 {
  struct { int x; };
  int arr[sizeof(x)]; // expected-error{{use of undeclared identifier 'x'}}
};
