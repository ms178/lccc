// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w -w --c
// RUN: %clang_cc1 %s -Wmicrosoft -verify -triple x86_64-pc-win32
// RUN: %clang_cc1 %s -Wmicrosoft -verify -triple x86_64-w64-mingw32
// RUN: %clang_cc1 %s -Wmicrosoft -verify -triple x86_64-pc-cygwin

// None of these should warn. stdcall is treated as equivalent to cdecl on
// x64.
// expected-no-diagnostics

int __stdcall f(void);
int __cdecl f(void) {
  return 0;
}
int __stdcall func_std(void);
int __thiscall func_this(void);
int __fastcall func_fast(void);
