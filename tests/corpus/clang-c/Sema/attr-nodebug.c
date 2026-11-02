// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -verify -fsyntax-only

int a __attribute__((nodebug));

void b(int p __attribute__((nodebug))) { // expected-warning {{'nodebug' attribute only applies to typedefs, functions, function pointers, Objective-C methods, and variables}}
  int b __attribute__((nodebug));
}

void t1(void) __attribute__((nodebug));

void t2(void) __attribute__((nodebug(2))); // expected-error {{'nodebug' attribute takes no arguments}}
