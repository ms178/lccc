// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -triple i686-unknown-unknown -verify %s
// RUN: %clang_cc1 -fsyntax-only -triple i686-- -verify %s
// expected-no-diagnostics

#if __is_target_arch(unknown)
  #error "mismatching arch"
#endif

// Unknown vendor is allowed.
#if !__is_target_vendor(unknown)
  #error "mismatching vendor"
#endif

// Unknown OS is allowed.
#if !__is_target_os(unknown)
  #error "mismatching OS"
#endif

// Unknown environment is allowed.
#if !__is_target_environment(unknown)
  #error "mismatching environment"
#endif

// Unknown variant OS is not allowed.
#if __is_target_variant_os(unknown)
  #error "mismatching OS"
#endif

// Unknown variant environment is not allowed.
#if __is_target_variant_environment(unknown)
  #error "mismatching environment"
#endif
