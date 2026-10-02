// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -verify %s
// expected-no-diagnostics

typedef int Object;

struct Object {int i1; } *P;

void foo(void) {
 struct Object { int i2; } *X;
  Object:
 {
    Object a;
 }
}

