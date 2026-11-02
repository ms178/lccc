// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c99: --c90
// RUN: %clang_cc1 -Wreturn-type -std=c99 -fsyntax-only -verify=c99 %s
// RUN: %clang_cc1 -Wreturn-type -std=c90 -fsyntax-only -verify=c90 %s

int foo(void) { return; } // c99-error {{non-void function 'foo' should return a value}}
                          // c90-error@-1 {{non-void function 'foo' should return a value}}
