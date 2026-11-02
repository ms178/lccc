// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

// PR50797
struct a {
  int xxx;
};

int g_107;
int g_108;
int g_109;

struct a g_999;

void b(void) { (g_910.xxx = g_910.xxx); } //expected-error 2{{use of undeclared identifier 'g_910'}}

void c(void) { (g_910.xxx = g_910.xxx1); } //expected-error 2{{use of undeclared identifier 'g_910'}}
