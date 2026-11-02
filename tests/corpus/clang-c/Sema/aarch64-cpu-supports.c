// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -fsyntax-only -triple aarch64-linux-gnu -verify %s

int test_aarch64_features(void) {
  char * ssbs2;
  // expected-error@+1 {{expression is not a string literal}}
  if (__builtin_cpu_supports(ssbs2))
    return 1;
  // expected-warning@+1 {{invalid cpu feature string}}
  if (__builtin_cpu_supports(""))
    return 2;
  // expected-warning@+1 {{invalid cpu feature string}}
  if (__builtin_cpu_supports("pmull128"))
    return 3;
  // expected-warning@+1 {{invalid cpu feature string}}
  if (__builtin_cpu_supports("sve2,sve"))
    return 4;
  // expected-warning@+1 {{invalid cpu feature string}}
  if (__builtin_cpu_supports("aes+sve2-pmull"))
    return 5;
  // expected-warning@+1 {{invalid cpu feature string}}
  if (__builtin_cpu_supports("default"))
    return 6;
  if (__builtin_cpu_supports(" ssbs + bti "))
    return 7;
  return 0;
}
