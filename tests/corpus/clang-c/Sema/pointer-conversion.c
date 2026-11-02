// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
//RUN: %clang_cc1 -fsyntax-only -verify %s

char * c;
char const ** c2 = &c; // expected-warning {{discards qualifiers in nested pointer types}}

typedef char dchar;
dchar *** c3 = &c2; // expected-warning {{discards qualifiers in nested pointer types}}

volatile char * c4;
char ** c5 = &c4; // expected-warning {{discards qualifiers in nested pointer types}}
