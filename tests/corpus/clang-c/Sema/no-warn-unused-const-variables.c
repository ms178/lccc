// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wunused-const-variable -x c-header -ffreestanding -verify %s
// RUN: %clang_cc1 -fsyntax-only -Wunused-const-variable -x c++-header -ffreestanding -verify %s
// expected-no-diagnostics
static const int unused[] = { 2, 3, 5, 7, 11, 13 };
