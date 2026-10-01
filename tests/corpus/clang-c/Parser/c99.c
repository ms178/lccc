// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c99: --c89
// RUN: %clang_cc1 -verify -pedantic -fsyntax-only -std=c99 %s
// RUN: %clang_cc1 -verify=expected,ext -pedantic -Wno-comment -fsyntax-only -std=c89 %s
// RUN: %clang_cc1 -verify=expected,ext -pedantic -fsyntax-only -x c++ %s

double _Imaginary foo; // ext-warning {{'_Imaginary' is a C99 extension}} \
                       // expected-error {{imaginary types are not supported}}
double _Complex bar; // ext-warning {{'_Complex' is a C99 extension}}

#if !defined(__cplusplus)
_Bool baz; // ext-warning {{'_Bool' is a C99 extension}}
#endif
