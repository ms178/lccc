// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple mips64 -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple mips64 -target-feature +soft-float -fsyntax-only -verify=softfloat %s

// expected-no-diagnostics

void test_f(float p) {
  float result = p;
  __asm__("" :: "f"(result)); // softfloat-error{{invalid input constraint 'f' in asm}}
}
