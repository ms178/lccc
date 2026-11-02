// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c: --c++03 --c++
// RUN: %clang_cc1 -fsyntax-only -verify -x c++ -std=c++98 %s

void a (void []()); // expected-error{{'type name' declared as array of functions}}
void b (void p[]()); // expected-error{{'p' declared as array of functions}}
void c (int &[]); // expected-error{{'type name' declared as array of references}}
void d (int &p[]); // expected-error{{'p' declared as array of references}}

