// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

typedef double vector4double __attribute__((__vector_size__(32)));
typedef float  vector8float  __attribute__((__vector_size__(32)));

vector8float foo1(vector4double x) {
  return __builtin_convertvector(x, vector8float);  // expected-error {{same number of elements}}
}

float foo2(vector4double x) {
  return __builtin_convertvector(x, float);  // expected-error {{second argument to __builtin_convertvector must be of vector type}}
}

vector8float foo3(double x) {
  return __builtin_convertvector(x, vector8float);  // expected-error {{must be a vector}}
}

float foo4(float x) {
  return __builtin_convertvector(x, float); // expected-error {{first argument to __builtin_convertvector must be a vector}}
}
