// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23: --c23
// RUN: %clang_cc1 -fsyntax-only -verify -std=c2x %s
// RUN: %clang_cc1 -fsyntax-only -verify -std=c23 %s
// expected-no-diagnostics

_Static_assert(__STDC_VERSION__ == 202311L, "Incorrect __STDC_VERSION__");
