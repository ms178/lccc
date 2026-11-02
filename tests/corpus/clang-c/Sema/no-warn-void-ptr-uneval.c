// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options: : --c89 --strict_gnu
// RUN: %clang_cc1 -fsyntax-only -Wno-unused-value -verify %s
// RUN: %clang_cc1 -fsyntax-only -Wno-unused-value -verify -std=gnu89 %s

// expected-no-diagnostics
void foo(void *vp) {
  sizeof(*vp);
  sizeof(*(vp));
  void inner(typeof(*vp));
}
