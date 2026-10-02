// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --ms_compatibility -w --c
// RUN: %clang_cc1 %s -fsyntax-only -Wno-unused-value -Wmicrosoft -verify -fms-compatibility -triple i386-pc-win32
int __stdcall f(void); /* expected-note {{previous declaration is here}} */

int __cdecl f(void) { /* expected-error {{function declared 'cdecl' here was previously declared 'stdcall'}} */
  return 0;
}
