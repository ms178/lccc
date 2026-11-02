// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options: 
// RUN: %clang_cc1 %s -triple avr-unknown-unknown -verify -fsyntax-only
struct a { int b; };

struct a test __attribute__((signal)); // expected-warning {{'signal' attribute only applies to functions}}

__attribute__((signal(12))) void foo(void) { } // expected-error {{'signal' attribute takes no arguments}}

__attribute__((signal)) void food(void) {}
