// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --ms_extensions --c
// RUN: %clang_cc1 -triple i686-pc-win32 -fms-extensions -verify %s

// Do not report that 'foo()' is redeclared without dllimport attribute.
// specified.

// expected-no-diagnostics
__declspec(dllimport) int __cdecl foo(void);
inline int __cdecl foo() { return 0; }
