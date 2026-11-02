// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --ms_extensions --c
// RUN: %clang_cc1 -triple armv7 -fms-extensions -fsyntax-only -ffreestanding -verify %s

unsigned int test_MoveFromCoprocessor(const unsigned int value) {
  return _MoveFromCoprocessor(value, 1, 2, 3, 4); // expected-error-re {{argument to {{.*}} must be a constant integer}}
}

void test_MoveToCoprocessor(const unsigned int value) {
  _MoveToCoprocessor(1, 2, value, 3, 4, 5); // expected-error-re {{argument to {{.*}} must be a constant integer}}
}
