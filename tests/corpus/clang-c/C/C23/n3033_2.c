// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -fsyntax-only -std=c23 -verify %s

#define H1(X, ...)       X __VA_OPT__(##) __VA_ARGS__ // expected-error {{'##' cannot appear at start of __VA_OPT__ argument}}

