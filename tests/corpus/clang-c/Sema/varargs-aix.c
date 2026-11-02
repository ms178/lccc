// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple powerpc-ibm-aix
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple powerpc64-ibm-aix
// expected-no-diagnostics

extern __builtin_va_list ap;
extern char *ap;
