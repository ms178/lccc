// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: not %clang_cc1 -fsyntax-only -verify %s
// PR2958
static struct foo s; // expected-error { tentative definition has type 'struct foo' that is never completed }
struct foo *p = &s;
