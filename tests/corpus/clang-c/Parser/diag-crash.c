// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

// Avoid preprocessor diag crash caused by a parser diag left in flight.

int foo: // expected-error {{expected ';' after top level declarator}}
#endif   // expected-error {{#endif without #if}}
