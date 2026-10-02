// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
// expected-no-diagnostics

int i;
int a[] = {0};
struct { int i; } s;

int *array[] = {&i, a, &s.i};

extern void f(void);
void (*f_addr)(void) = &f;
