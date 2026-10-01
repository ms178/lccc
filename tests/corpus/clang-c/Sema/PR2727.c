// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c90: --c99
// RUN: %clang_cc1 -verify -fsyntax-only -std=c90 %s
// RUN: %clang_cc1 -verify -fsyntax-only -std=c99 %s
// expected-no-diagnostics

int f (int x)
{
  // sizeof applied to a type should not delete the type.
  return sizeof (int[x]);
}
