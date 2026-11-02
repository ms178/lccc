// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c99
// RUN: %clang_cc1 -fsyntax-only -verify -std=c99 %s

int f (int z)
{ 
   if (z > (int) sizeof (enum {a, b}))
      return a;
   return b; // expected-error{{use of undeclared identifier}}
}
