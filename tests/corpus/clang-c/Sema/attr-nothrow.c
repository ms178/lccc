// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -verify
// RUN: %clang_cc1 %s -ast-dump | FileCheck %s
// expected-no-diagnostics

// PR42113: The following caused an assertion in mergeFunctionTypes
// because it causes one side to have an exception specification, which
// isn't typically supported in C.
void PR42113a();
void PR42113a(void) __attribute__((nothrow));
// CHECK: FunctionDecl {{.*}} PR42113a
// CHECK: FunctionDecl {{.*}} PR42113a
// CHECK: NoThrowAttr
void PR42113b() __attribute__((nothrow));
// CHECK: FunctionDecl {{.*}} PR42113b
// CHECK: NoThrowAttr
 __attribute__((nothrow)) void PR42113c();
// CHECK: FunctionDecl {{.*}} PR42113c
// CHECK: NoThrowAttr
