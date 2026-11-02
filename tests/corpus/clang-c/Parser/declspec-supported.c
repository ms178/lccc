// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --ms_extensions --c
// RUN: %clang_cc1 -triple i386-pc-unknown -fsyntax-only -fms-extensions -verify %s
// RUN: %clang_cc1 -triple i386-pc-unknown -fsyntax-only -fdeclspec -verify %s
// expected-no-diagnostics

__declspec(naked) void f(void) {}

struct S {
  __declspec(property(get=Getter, put=Setter)) int X;
  int Y;
};
