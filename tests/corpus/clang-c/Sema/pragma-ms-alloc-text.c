// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --ms_extensions --c
// RUN: %clang_cc1 -fms-extensions -fsyntax-only -verify %s

void foo();
#pragma alloc_text("hello", foo) // no-error
void foo() {}

static void foo1();
#pragma alloc_text("hello", foo1) // no-error
void foo1() {}

int foo2;
#pragma alloc_text(c, foo2) // expected-error {{'#pragma alloc_text' is applicable only to functions}}
