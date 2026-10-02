// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only %s -verify
// RUN: %clang_cc1 -fsyntax-only %s -fexperimental-new-constant-interpreter -verify
// expected-no-diagnostics

struct S {
 int one;
 int two;
};

struct S const foo(void);


struct S tmp;

void priv_sock_init(void) {
  tmp = (struct S)foo();
}
