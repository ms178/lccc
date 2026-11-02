// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -pedantic-errors -Wno-empty-translation-unit -verify 
// PR2045

#define b
/* expected-error {{extra tokens at end of #undef directive}} */ #undef a b
