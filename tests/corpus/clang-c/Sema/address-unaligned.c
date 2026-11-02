// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --ms_extensions --c
// RUN: %clang_cc1 -fsyntax-only -fms-extensions -verify %s
// expected-no-diagnostics

typedef
struct __attribute__((packed)) S1 {
  char c0;
  int x;
  char c1;
} S1;

void bar(__unaligned int *);

void foo(__unaligned S1* s1)
{
    bar(&s1->x);
}
