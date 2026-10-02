// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -std=c2y -verify %s
// RUN: %clang_cc1 -std=c2y -verify -fexperimental-new-constant-interpreter %s
// expected-no-diagnostics

void gh152826(char (*a)[*][5], int (*x)[_Countof(*a)]);
void more_likely_in_practice(unsigned long size_one, int (*a)[*][5], int b[_Countof(*a)]);
void f(int (*x)[*][1][*][2][*][*][3][*], int q[_Countof(*x)]);
