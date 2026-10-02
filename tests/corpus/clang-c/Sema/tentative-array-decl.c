// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -verify %s
// RUN: %clang_cc1 -verify=good -Wno-tentative-definition-array %s
// good-no-diagnostics

int foo[]; // expected-warning {{tentative array definition assumed to have one element}}
