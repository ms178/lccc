// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -Wno-return-type -fsyntax-only -verify
// expected-no-diagnostics

int t14(void) {
  return;
}

void t15(void) {
  return 1;
}
