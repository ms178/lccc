// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --ms_extensions --c
// RUN: %clang_cc1 %s -verify -fsyntax-only -fms-extensions

int __unaligned * p1; // expected-note {{previous definition is here}}
int * p1; // expected-error {{redefinition of 'p1' with a different type: 'int *' vs '__unaligned int *'}}
