// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s
// expected-no-diagnostics

// PR7477
enum __attribute__((packed)) E {
  Ea, Eb, Ec, Ed
};

void test_E(enum E e) {
  switch (e) {
  case Ea:
  case Eb:
  case Ec:
  case Ed:
    break;
  }
}
