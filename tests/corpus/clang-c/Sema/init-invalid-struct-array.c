// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify -fsyntax-only

struct S {
  Unknown u; // expected-error {{unknown type name 'Unknown'}}
  int i;
};
// Should not crash
struct S s[] = {[0].i = 0, [1].i = 1, {}};
