// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

// expected-no-diagnostics

typedef __SIZE_TYPE__ size_t;
void *memset(void *, int, size_t);

typedef struct {
  int a;
} S;

void test() {
  S s;
  __auto_type dstptr = &s;
  memset(dstptr, 0, sizeof(s));
}
