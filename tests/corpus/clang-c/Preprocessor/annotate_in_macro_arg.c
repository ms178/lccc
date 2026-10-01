// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -verify %s
#define M1() // expected-note{{macro 'M1' defined here}}

M1( // expected-error{{unterminated function-like macro invocation}}

#if M1() // expected-error{{expected value in expression}}
#endif
#pragma pack()
