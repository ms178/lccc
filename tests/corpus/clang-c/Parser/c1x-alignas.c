// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c11: --c99
// RUN: %clang_cc1 -std=c11 -fsyntax-only -verify %s
// RUN: not %clang_cc1 -std=c99 -pedantic -fsyntax-only %s 2>&1 | FileCheck -check-prefix=CHECK-EXT %s

_Alignas(4) char c1;
unsigned _Alignas(long) char c2;
char _Alignas(16) c3;

char c4 _Alignas(32); // expected-error {{expected ';' after top level declarator}} expected-warning {{declaration does not declare anything}}

char _Alignas(_Alignof(int)) c5;

_Alignas(int) struct c6; // expected-warning {{'_Alignas' attribute ignored}}
// CHECK-EXT: '_Alignas' is a C11 extension
// CHECK-EXT: '_Alignof' is a C11 extension
