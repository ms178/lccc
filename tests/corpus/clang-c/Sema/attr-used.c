// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -verify -fsyntax-only -Wno-private-extern %s

extern int l0 __attribute__((used)); // expected-warning {{'used' attribute ignored on a non-definition declaration}}
__private_extern__ int l1 __attribute__((used)); // expected-warning {{'used' attribute ignored on a non-definition declaration}}

struct __attribute__((used)) s { // expected-warning {{'used' attribute only applies to variables with non-local storage, functions, and Objective-C methods}}
  int x;
};

int a __attribute__((used));

static void __attribute__((used)) f0(void) {
}

void f1(void) {
  static int a __attribute__((used));
  int b __attribute__((used)); // expected-warning {{'used' attribute only applies to variables with non-local storage, functions, and Objective-C methods}}
}

static void __attribute__((used)) f0(void);
