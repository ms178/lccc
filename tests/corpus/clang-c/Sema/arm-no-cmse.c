// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple thumbv8m.base-none-eabi -verify %s

typedef void (*callback_ns_1t)(void)
  __attribute__((cmse_nonsecure_call)); // expected-warning{{'cmse_nonsecure_call' attribute ignored}}

void f(void)
  __attribute__((cmse_nonsecure_entry)) {} // expected-warning{{'cmse_nonsecure_entry' attribute ignored}}
