// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -verify -fsyntax-only %s
// PR4214
typedef void vt;
void (*func_ptr)(vt my_vt); // expected-error {{argument may not have 'void' type}}
