// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
// Ensure we see the error from PP and do not see errors from the parser.

// expected-error@+1{{'#' is not followed by a macro parameter}}
#define INVALID() #B 10+10
