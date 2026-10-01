// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  -w --c
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple x86_64-pc-win32
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple x86_64-uefi

void __attribute__((sysv_abi)) foo(int a, ...) {
  __builtin_va_list ap;
  __builtin_va_start(ap, a); // expected-error {{'va_start' used in System V ABI function}}
}
