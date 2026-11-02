// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options: : --c89 --strict_gnu
// RUN: %clang_cc1 %s -std=gnu89 -pedantic -fsyntax-only -verify

int f(int restrict);

void main(void) {} // expected-warning {{return type of 'main' is not 'int'}} expected-note {{change return type to 'int'}}
