// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple i686-pc-linux-gnu
// expected-no-diagnostics

// PR 4142 - support glibc extension to printf: '%m' (which prints strerror(errno)).
int printf(char const*,...);
void percentm(void) {
  printf("%m");
}
