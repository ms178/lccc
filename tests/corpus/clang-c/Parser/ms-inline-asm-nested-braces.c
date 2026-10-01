// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// REQUIRES: x86-registered-target
// RUN: %clang_cc1 %s -triple i386-apple-darwin10 -verify -fasm-blocks

int t_fail(void) { // expected-note {{to match this}}
  __asm
  { // expected-note {{to match this}}
    { // expected-note {{to match this}}
      {
      } // expected-error 3 {{expected}}
