// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c99
// RUN: %clang_cc1 -fsyntax-only -verify %s -std=c99

int f3(y, x,       // expected-warning {{a function definition without a prototype is deprecated in all versions of C and is not supported in C23}}
       x)          // expected-error {{redefinition of parameter}}
  int y, 
      x,           // expected-note {{previous declaration is here}}
      x;           // expected-error {{redefinition of parameter}}
{
  return x + y; 
} 

void f4(void) { 
  f3 (1, 1, 2, 3, 4); // expected-warning{{too many arguments}}
}

