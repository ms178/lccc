// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple i386-apple-darwin9

void foo(int a, ...) {
  __builtin_ms_va_start((void *)0, a); // expected-error {{this builtin is only available on x86-64 and aarch64 targets}}
}
