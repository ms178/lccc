// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple i386-linux-android -fsyntax-only -verify %s
// expected-no-diagnostics

extern int a1_0[sizeof(long double) == 8 ? 1 : -1];
extern int a1_i[__alignof(long double) == 4 ? 1 : -1];

