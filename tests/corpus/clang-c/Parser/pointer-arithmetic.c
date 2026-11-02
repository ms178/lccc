// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

int *test1(int *a)         { return a + 1; }
int *test2(int *a)         { return 1 + a; }
int *test3(int *a)         { return a - 1; }
int  test4(int *a, int *b) { return a - b; }

int  test5(int *a, int *b) { return a + b; } /* expected-error {{invalid operands}} */
int *test6(int *a)         { return 1 - a; } /* expected-error {{invalid operands}} */
