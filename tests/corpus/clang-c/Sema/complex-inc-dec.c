// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99
// RUN: %clang_cc1 -verify -pedantic -std=c99 %s

void func(void) {
  _Complex float cf;
  _Complex double cd;
  _Complex long double cld;

  ++cf;  // expected-warning {{'++' on an object of complex type is a C2y extension}}
  ++cd;  // expected-warning {{'++' on an object of complex type is a C2y extension}}
  ++cld; // expected-warning {{'++' on an object of complex type is a C2y extension}}

  --cf;  // expected-warning {{'--' on an object of complex type is a C2y extension}}
  --cd;  // expected-warning {{'--' on an object of complex type is a C2y extension}}
  --cld; // expected-warning {{'--' on an object of complex type is a C2y extension}}

  cf++;  // expected-warning {{'++' on an object of complex type is a C2y extension}}
  cd++;  // expected-warning {{'++' on an object of complex type is a C2y extension}}
  cld++; // expected-warning {{'++' on an object of complex type is a C2y extension}}

  cf--;  // expected-warning {{'--' on an object of complex type is a C2y extension}}
  cd--;  // expected-warning {{'--' on an object of complex type is a C2y extension}}
  cld--; // expected-warning {{'--' on an object of complex type is a C2y extension}}
}

