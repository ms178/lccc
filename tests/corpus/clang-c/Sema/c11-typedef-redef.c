// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c11
// RUN: %clang_cc1 -std=c11 %s -verify

typedef int type;
typedef type type;
typedef int type;

void f(int N) {
  typedef int type2;
  typedef type type2;
  typedef int type2;

  typedef int vla[N]; // expected-note{{previous definition is here}}
  typedef int vla[N]; // expected-error{{redefinition of typedef for variably-modified type 'int[N]'}}

  typedef int vla2[N];
  typedef vla2 vla3; // expected-note{{previous definition is here}}
  typedef vla2 vla3; // expected-error{{redefinition of typedef for variably-modified type 'vla2' (aka 'int[N]')}}
}
