// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple=x86_64-pc-linux-gnu -fsyntax-only -Wenum-conversion -verify %s
// RUN: %clang_cc1 -triple=x86_64-pc-linux-gnu -fsyntax-only -Wconversion -verify %s

// Signed enums
enum SE1 { N1 = -1 };
enum SE2 { N2 = -2 };
// Unsigned unums
enum UE1 { P1 };
enum UE2 { P2 };

enum UE2 f1(enum UE1 E) {
  return E; // expected-warning {{implicit conversion from enumeration type 'enum UE1' to different enumeration type 'enum UE2'}}
}

enum SE1 f2(enum UE1 E) {
  return E; // expected-warning {{implicit conversion from enumeration type 'enum UE1' to different enumeration type 'enum SE1'}}
}

enum UE1 f3(enum SE1 E) {
  return E; // expected-warning {{implicit conversion from enumeration type 'enum SE1' to different enumeration type 'enum UE1'}}
}

enum SE2 f4(enum SE1 E) {
  return E; // expected-warning {{implicit conversion from enumeration type 'enum SE1' to different enumeration type 'enum SE2'}}
}
