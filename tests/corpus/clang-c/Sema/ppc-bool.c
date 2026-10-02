// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple powerpc-apple-macosx10.4.0 -verify -fsyntax-only %s
// expected-no-diagnostics
extern __typeof(+(_Bool)0) should_be_int;
extern int should_be_int;
