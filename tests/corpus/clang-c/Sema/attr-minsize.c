// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

int foo(void) __attribute__((__minsize__));

int var1 __attribute__((__minsize__)); // expected-error{{'__minsize__' attribute only applies to functions and Objective-C methods}}
