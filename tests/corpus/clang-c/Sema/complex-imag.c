// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -verify %s

void f1(void) {
  int a = 1;
  int b = __imag a;
  int *c = &__real a;
  int *d = &__imag a; // expected-error {{cannot take the address of an rvalue of type 'int'}}
}

void f2(void) {
  _Complex int a = 1;
  int b = __imag a;
  int *c = &__real a;
  int *d = &__imag a;
}

void f3(void) {
  double a = 1;
  double b = __imag a;
  double *c = &__real a;
  double *d = &__imag a; // expected-error {{cannot take the address of an rvalue of type 'double'}}
}

void f4(void) {
  _Complex double a = 1;
  double b = __imag a;
  double *c = &__real a;
  double *d = &__imag a;
}

// PR69218
int f5(void) {
  float _Complex a;
  return (0 < &__real__ a) && (0 < &__imag__ a);
}
