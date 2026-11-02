// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -Wall -ffreestanding -fsyntax-only -fwrapv -verify %s

int test(void) {
  int i;
  i = -1 << 1; // no-warning
  return i;
}

// expected-no-diagnostics
