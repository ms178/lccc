// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple x86_64-apple-darwin9
// expected-no-diagnostics

// Do not warn about undefined behavior of parameter argument types in
// unreachable code in a macro.
#define VA_ARG_RDAR12322000(Marker, TYPE)         ((sizeof (TYPE) < sizeof (UINTN_RDAR12322000)) ? (TYPE)(__builtin_va_arg (Marker, UINTN_RDAR12322000)) : (TYPE)(__builtin_va_arg (Marker, TYPE)))

// 64-bit system
typedef unsigned long long  UINTN_RDAR12322000;

int test_VA_ARG_RDAR12322000 (__builtin_va_list Marker)
{
  return VA_ARG_RDAR12322000 (Marker, short); // no-warning
}