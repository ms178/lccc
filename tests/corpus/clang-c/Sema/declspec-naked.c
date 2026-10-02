// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  -w -w -w --c
// RUN: %clang_cc1 -triple i686-unknown-windows-msvc -fsyntax-only -fdeclspec -verify %s
// RUN: %clang_cc1 -triple thumbv7-unknown-windows-msvc -fsyntax-only -fdeclspec -verify %s
// RUN: %clang_cc1 -triple x86_64-unknown-windows-msvc -fsyntax-only -fdeclspec -verify %s
#if defined(_M_IX86) || defined(_M_ARM)
// CHECK: expected-no-diagnostics
#endif

void __declspec(naked) f(void) {}
#if !defined(_M_IX86) && !defined(_M_ARM)
// expected-error@-2{{'naked' attribute is not supported on 'x86_64'}}
#endif
