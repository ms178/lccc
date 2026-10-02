// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -triple arm64-apple-ios11 -verify %s
// expected-no-diagnostics

#if !__is_target_arch(arm64) || !__is_target_arch(aarch64)
  #error "mismatching arch"
#endif

#if __is_target_arch(aarch64_be)
  #error "mismatching arch"
#endif
