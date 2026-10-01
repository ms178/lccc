// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  -w --c --target linux_aarch64
// RUN: %clang_cc1 -triple aarch64-unknown-unknown -fexperimental-max-bitint-width=1024 -fsyntax-only -verify %s

void f() {
  _Static_assert(__BITINT_MAXWIDTH__ == 1024, "Macro value is unexpected.");

  _BitInt(1024) a;
  unsigned _BitInt(1024) b;

  _BitInt(8388609) c;                // expected-error {{signed _BitInt of bit sizes greater than 1024 not supported}}
  unsigned _BitInt(0xFFFFFFFFFF) d; // expected-error {{unsigned _BitInt of bit sizes greater than 1024 not supported}}
}
