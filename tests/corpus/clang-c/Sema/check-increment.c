// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
// RUN: %clang_cc1 -fsyntax-only -verify %s -fexperimental-new-constant-interpreter
// expected-no-diagnostics

int printf(const char *, ...);
typedef int *pint;
int main(void) {
   int a[5] = {0};
   pint p = a;
   p++;
   printf("%d\n", *p);
}
