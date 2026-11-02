// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --ms_compatibility -w --ms_compatibility --c --target linux_aarch64
// RUN: %clang_cc1 -triple thumbv7-windows -fms-compatibility -fsyntax-only -verify %s
// RUN: %clang_cc1 -triple aarch64-windows -fms-compatibility -fsyntax-only -verify %s

int __cdecl cdecl(int a, int b, int c, int d) { // expected-no-diagnostics
  return a + b + c + d;
}

float __stdcall stdcall(float a, float b, float c, float d) { // expected-no-diagnostics
  return a + b + c + d;
}
