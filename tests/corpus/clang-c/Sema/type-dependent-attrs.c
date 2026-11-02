// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -std=c23 -fsyntax-only -verify %s

int open() { return 0; }
void close(typeof(open()) *) {}

void cleanup_attr() {
  int fd_int [[gnu::cleanup(close)]] = open();
  auto fd_auto [[gnu::cleanup(close)]] = open();
  float fd_invalid [[gnu::cleanup(close)]] = open(); // expected-error {{'cleanup' function 'close' parameter has type 'typeof (open()) *' (aka 'int *') which is incompatible with type 'float *'}}
}
