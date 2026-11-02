// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple arm-none-eabi  -fsyntax-only -verify %s

int __attribute__((target("sse4.2"))) redecl1(void) { return 1; }
//expected-error@+2 {{function multiversioning is not supported on the current target}}
//expected-note@-2 {{previous declaration is here}}
int __attribute__((target("avx")))  redecl1(void) { return 2; }

//expected-error@+1 {{function multiversioning is not supported on the current target}}
int __attribute__((target("default"))) with_def(void) { return 1;}
