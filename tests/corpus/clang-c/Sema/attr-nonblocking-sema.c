// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c89
// RUN: %clang_cc1 -fsyntax-only -verify -std=c89 %s

// Tests for a few cases involving C functions without prototypes.

void noproto() __attribute__((nonblocking)) // expected-error {{'nonblocking' function must have a prototype}}
{
}

// This will succeed
void noproto(void) __attribute__((blocking));

// A redeclaration isn't any different - a prototype is required.
void f1(void);
void f1() __attribute__((nonblocking)); // expected-error {{'nonblocking' function must have a prototype}}
