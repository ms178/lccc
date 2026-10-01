// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options: 
// RUN: %clang_cc1 %s -triple avr-unknown-unknown -verify -fsyntax-only
struct a { int b; };

struct a test __attribute__((interrupt)); // expected-warning {{'interrupt' attribute only applies to functions}}

__attribute__((interrupt(12))) void foo(void) { } // expected-error {{'interrupt' attribute takes no arguments}}

__attribute__((interrupt)) void food(void) {}
