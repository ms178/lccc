// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -fdefer-ts -verify=enabled %s
// RUN: %clang_cc1 -fsyntax-only -verify=disabled %s
// RUN: %clang_cc1 -x c++ -fsyntax-only -fdefer-ts -verify=disabled %s
// RUN: %clang_cc1 -x c++ -fsyntax-only -verify=disabled %s
// enabled-no-diagnostics
#if __STDC_DEFER_TS25755__ != 1
// disabled-error@+1 {{Should have defined __STDC_DEFER_TS25755__ to 1}}
#  error Should have defined __STDC_DEFER_TS25755__ to 1
#endif
