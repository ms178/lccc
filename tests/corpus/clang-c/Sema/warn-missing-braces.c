// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wmissing-braces -verify %s

int a[2][2] = { 0, 1, 2, 3 }; // expected-warning{{suggest braces}} expected-warning{{suggest braces}}
