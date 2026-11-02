// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fexperimental-strict-floating-point -fsyntax-only -Wignored-pragmas -verify %s

#pragma STDC FENV_ROUND ON   // expected-warning {{invalid or unsupported rounding mode}}

float func_01(int x, float y) {
  if (x)
    return y + 2;
  #pragma STDC FENV_ROUND FE_DOWNWARD // expected-error{{'#pragma STDC FENV_ROUND' can only appear at file scope or at the start of a compound statement}}
                                      // expected-warning@-1{{pragma STDC FENV_ROUND is not supported}}
  return x + y;
}
