// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -triple arm64-apple-ios -fsyntax-only -verify -fptrauth-intrinsics %s
// RUN: %clang_cc1 -triple aarch64-linux-gnu -fsyntax-only -verify -fptrauth-intrinsics %s

#if __aarch64__
#define VALID_DATA_KEY 2
#else
#error Provide these constants if you port this test
#endif

int * __ptrauth(VALID_DATA_KEY) valid0;

typedef int *intp;

int nonConstantGlobal = 5;

__ptrauth int invalid0; // expected-error{{expected '('}}
__ptrauth() int invalid1; // expected-error{{expected expression}}
int * __ptrauth(VALID_DATA_KEY, 1, 1000, 12) invalid12; // expected-error{{qualifier must take between 1 and 3 arguments}}
