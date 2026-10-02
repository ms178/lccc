// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99: --c11
// Check for warnings in non-C11 mode:
// RUN: %clang_cc1 -fsyntax-only -std=c99 -verify -Wc11-extensions %s

// Expect no warnings in C11 mode:
// RUN: %clang_cc1 -fsyntax-only -std=c11 -pedantic -Werror %s

struct s {
  int a;
  struct { // expected-warning{{anonymous structs are a C11 extension}}
    int b;
  };
};

struct t {
  int a;
  union { // expected-warning{{anonymous unions are a C11 extension}}
    int b;
  };
};
