// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify -fsyntax-only

int a __attribute__((noinline)); // expected-warning {{'noinline' attribute only applies to functions and statements}}

void t1(void) __attribute__((noinline));

void t2(void) __attribute__((noinline(2))); // expected-error {{'noinline' attribute takes no arguments}}

