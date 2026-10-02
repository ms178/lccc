// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

typedef _Atomic char atomic_char;

atomic_char counter;

char load_plus_one(void) {
  return ({counter;}) + 1; // no crash
}

char type_of_stmt_expr(void) {
  typeof(({counter;})) y = ""; // expected-error-re {{incompatible pointer to integer conversion initializing 'typeof (({{{.*}}}))' (aka 'char') with an expression of type 'char[1]'}}
  return y;
}
