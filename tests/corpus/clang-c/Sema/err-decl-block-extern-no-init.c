// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple x86_64-unknown-unknown -fsyntax-only -verify %s
static int x;

void foo(void)
{
    extern int x = 1; // expected-error {{declaration of block scope identifier with linkage cannot have an initializer}}
}

int y;

void bar(void)
{
    extern int y = 1; // expected-error {{declaration of block scope identifier with linkage cannot have an initializer}}

}
