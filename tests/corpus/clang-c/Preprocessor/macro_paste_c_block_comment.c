// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -Eonly -verify

// expected-error@9 {{EOF}}
#define COMM / ## *
COMM // expected-error {{pasting formed '/*', an invalid preprocessing token}}

// Demonstrate that an invalid preprocessing token
// doesn't swallow the rest of the file...
#error EOF
