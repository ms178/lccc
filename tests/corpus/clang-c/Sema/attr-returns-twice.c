// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify -fsyntax-only

int a __attribute__((returns_twice)); // expected-warning {{'returns_twice' attribute only applies to functions}}

__attribute__((returns_twice)) void t0(void) {
}

void t1(void) __attribute__((returns_twice));

void t2(void) __attribute__((returns_twice(2))); // expected-error {{'returns_twice' attribute takes no arguments}}

typedef void (*t3)(void) __attribute__((returns_twice)); // expected-warning {{'returns_twice' attribute only applies to functions}}
