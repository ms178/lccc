// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -triple  powerpc-ibm-aix7.1.0.0 -verify %s

int main(void) {
  if (__builtin_cpu_is("power8")) // expected-error {{this builtin is available only on AIX 7.2 and later operating systems}}
    return 1;

  if (__builtin_cpu_supports("power8")) // expected-error {{this builtin is available only on AIX 7.2 and later operating systems}}
    return 1;
}
