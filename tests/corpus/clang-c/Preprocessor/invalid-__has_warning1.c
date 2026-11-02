// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -verify %s

// These must be the last lines in this test.
// expected-error@+1{{unterminated}} expected-error@+1 2{{expected}}
int i = __has_warning(
