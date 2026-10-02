// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

int main(void) {
  int line = __builtin_LINE();
  __builtin_LINE(42); // expected-error {{expected ')'}}
  __builtin_LINE(double); // expected-error {{expected ')'}}

  int column = __builtin_COLUMN();
  __builtin_COLUMN(42); // expected-error {{expected ')'}}
  __builtin_COLUMN(double); // expected-error {{expected ')'}}

  const char *func = __builtin_FUNCTION();
  __builtin_FUNCTION(42); // expected-error {{expected ')'}}
  __builtin_FUNCTION(double); // expected-error {{expected ')'}}

  const char *file = __builtin_FILE();
  __builtin_FILE(42); // expected-error {{expected ')'}}
  __builtin_FILE(double); // expected-error {{expected ')'}}
}
