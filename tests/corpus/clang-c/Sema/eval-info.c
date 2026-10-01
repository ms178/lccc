// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 %s -fsyntax-only -triple x86_64-unknown-windows-msvc -verify

// expected-no-diagnostics

// Make sure the new constant interpolator is not enabled unintentionally
// to cause assertion.
typedef enum x {
  a = 1,
} x;
