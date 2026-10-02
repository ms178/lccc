// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify -Wattributes %s

int x __attribute__((foobar)); // expected-warning {{unknown attribute 'foobar' ignored}}
void z(void) __attribute__((bogusattr)); // expected-warning {{unknown attribute 'bogusattr' ignored}}
