// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -E -verify %s

#define r_paren )
#if defined( x r_paren  // expected-error {{missing ')' after 'defined'}} \
                        // expected-note {{to match this '('}}
#endif
