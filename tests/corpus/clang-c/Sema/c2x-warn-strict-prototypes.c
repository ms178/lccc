// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 -fsyntax-only -Wstrict-prototypes -verify -std=c2x %s
// RUN: %clang_cc1 -fsyntax-only -Wstrict-prototypes -verify -fno-knr-functions %s
// expected-no-diagnostics

void foo();
void bar() {}

void baz(void);
void baz() {}
