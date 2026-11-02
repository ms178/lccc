// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
  // RUN: %clang_cc1 -fsyntax-only -fblocks -verify %s

  // Test that sentinel attribute on block variables doesn't crash
  void foo(void) {
    void (^a)() __attribute__((__sentinel__)) = {}; // expected-warning {{'sentinel' attribute requires named arguments}}
  }
