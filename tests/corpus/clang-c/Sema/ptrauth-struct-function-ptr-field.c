// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple arm64-apple-ios -x c   -fsyntax-only -verify -fptrauth-intrinsics %s -fexperimental-new-constant-interpreter
// RUN: %clang_cc1 -triple arm64-apple-ios -x c++ -fsyntax-only -verify -fptrauth-intrinsics %s -fexperimental-new-constant-interpreter

struct Foo {
  void (*f)(int) __ptrauth(1,1,1);
  // expected-error@-1 {{expected ';' at end of declaration list}}
};
