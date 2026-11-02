// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c11
// RUN: %clang_cc1 %s -verify -fsyntax-only -std=c11

int x(void) { e: b: ; return &&e - &&b < x; } // expected-warning {{ordered comparison between pointer and integer}}
