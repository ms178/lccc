// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c17
// RUN: %clang_cc1 -fsyntax-only -verify -std=c17 %s
// RUN: %clang_cc1 -fsyntax-only -verify -std=c18 %s
// expected-no-diagnostics

_Static_assert(__STDC_VERSION__ == 201710L, "Incorrect __STDC_VERSION__");
