// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

struct a { struct { int b; } x[2]; };

int a = __builtin_offsetof(struct a, x; // expected-error{{expected ')'}} expected-note{{to match this '('}}
// FIXME: This actually shouldn't give an error
int b = __builtin_offsetof(struct a, x->b); // expected-error{{expected ')'}} expected-note{{to match this '('}}
