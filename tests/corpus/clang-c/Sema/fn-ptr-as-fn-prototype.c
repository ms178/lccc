// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -ast-print %s | FileCheck %s

// This testcase checks the functionality of 
// Sema::ActOn{Start,End}FunctionDeclarator, specifically checking that
// ActOnEndFunctionDeclarator is called after the typedef so the enum
// is in the global scope, not the scope of f().

// CHECK: typedef void (*g)();
typedef void (*g) ();
// CHECK: enum
enum {
  k = -1
};
// CHECK: void f() {
void f() {}
