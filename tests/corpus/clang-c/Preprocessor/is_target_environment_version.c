// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 -fsyntax-only -triple x86_64-pc-windows-msvc18.0.0 -verify %s
// expected-no-diagnostics

#if !__is_target_environment(msvc)
  #error "mismatching environment"
#endif
