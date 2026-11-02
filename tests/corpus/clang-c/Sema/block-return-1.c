// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s -verify -fblocks

int j;
void foo(void) {
  ^ (void) { if (j) return 1; }(); // expected-error {{non-void block does not return a value in all control paths}}
}
