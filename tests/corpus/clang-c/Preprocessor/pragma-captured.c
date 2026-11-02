// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E %s | FileCheck %s

// Test pragma clang __debug captured, for Captured Statements

void test1()
{
  #pragma clang __debug captured
  {
  }
// CHECK: void test1()
// CHECK: {
// CHECK: #pragma clang __debug captured
}
