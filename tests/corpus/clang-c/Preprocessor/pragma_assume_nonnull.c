// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
// RUN: %clang_cc1 -E %s | FileCheck %s

// CHECK: #pragma clang assume_nonnull begin
#pragma clang assume_nonnull begin

int bar(int * ip) { return *ip; }

// CHECK: #pragma clang assume_nonnull end
#pragma clang assume_nonnull end

int foo(int * _Nonnull ip) { return *ip; }

int main(void) {
   return bar(0) + foo(0); // expected-warning 2 {{null passed to a callee that requires a non-null argument}}
}
