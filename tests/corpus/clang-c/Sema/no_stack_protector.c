// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -fsyntax-only -verify -std=c23 %s

[[gnu::no_stack_protector]] void test1(void) {}
[[clang::no_stack_protector]] void test2(void) {}

void __attribute__((no_stack_protector)) foo(void) {}
int __attribute__((no_stack_protector)) var; // expected-warning {{'no_stack_protector' attribute only applies to functions}}
void  __attribute__((no_stack_protector(2))) bar(void) {} // expected-error {{'no_stack_protector' attribute takes no arguments}}
