// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple x86_64-apple-darwin  -fsyntax-only -verify %s

void g(void) {}

void f(void) __attribute__((alias("g"))); //expected-error {{aliases are not supported on darwin}}
