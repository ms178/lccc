// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify -fsyntax-only

int a __attribute__((noduplicate)); // expected-warning {{'noduplicate' attribute only applies to functions}}

void t1(void) __attribute__((noduplicate));

void t2(void) __attribute__((noduplicate(2))); // expected-error {{'noduplicate' attribute takes no arguments}}

