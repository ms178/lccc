// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only

typedef float CGFloat;

extern void func(CGFloat);
void foo(int dir, int n, int tindex) {
  const float PI = 3.142;
  CGFloat cgf = 3.4;

  float ang = (float) tindex * (-dir*2.0f*PI/n);
  func((CGFloat)cgf/65535.0f);
}
