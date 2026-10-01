// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 -std=c2x -fsyntax-only -verify -pedantic %s

// Test that we get the extension warning when appropriate and that it shows up
// in the right location.
void test(void) {
  (void)_Generic(
	  int,  // expected-warning {{passing a type argument as the first operand to '_Generic' is a C2y extension}}
	  int : 0);
  (void)_Generic(
	  12,
	  int : 0);
}

