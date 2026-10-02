// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify -Wno-strict-prototypes
int f(int a) { return 0; } // expected-note {{previous definition is here}}
int f(int);
int f(int a) { return 0; } // expected-error {{redefinition of 'f'}}

int foo(x) { // expected-error {{parameter 'x' was not declared, defaults to 'int'; ISO C99 and later do not support implicit int}}
  return 0;
}
int x = 1;

extern inline int g(void) { return 0; } // expected-note{{previous definition}}
int g(void) { return 0; } // expected-error{{redefinition of a 'extern inline' function 'g' is not supported in C99 mode}}
