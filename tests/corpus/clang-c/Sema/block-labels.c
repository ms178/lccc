// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify -fblocks -fsyntax-only

void xx(void);

int a(void) { 
  A:
  
  if (1) xx();
  return ^{
         A: return 1;
       }();
}
int b(void) { 
  A: return ^{int a; A:return 1;}();
}

int d(void) { 
  A: return ^{int a; A: a = ^{int a; A:return 1;}() + ^{int b; A:return 2;}(); return a; }();
}

int c(void) { 
  goto A;     // expected-error {{use of undeclared label 'A'}}
  return ^{
       A:
        return 1;
     }();
}
