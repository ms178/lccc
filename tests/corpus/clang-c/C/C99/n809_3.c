// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99
// RUN: %clang_cc1 -emit-llvm -std=c99 %s -o - | FileCheck %s

// Demonstrate that statics are properly zero initialized.
static _Complex float f_global;
void func(void) {
  static _Complex double d_local;
  d_local = f_global;
}

// CHECK-DAG: @func.d_local = internal global { double, double } zeroinitializer
// CHECK-DAG: @f_global = internal global { float, float } zeroinitializer

