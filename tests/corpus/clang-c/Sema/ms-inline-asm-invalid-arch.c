// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -triple powerpc64-unknown-linux-gnu -fasm-blocks -verify -fsyntax-only

void f(void) {
  __asm nop // expected-error {{unsupported architecture 'powerpc64' for MS-style inline assembly}}
}
