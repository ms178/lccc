// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -verify -pedantic -fsyntax-only -Wno-gnu-statement-expression

int stmtexpr_fn(void);
void stmtexprs(int i) {
  __builtin_assume( ({ 1; }) ); // no warning about "side effects"
  __builtin_assume( ({ if (i) { (void)0; }; 42; }) ); // no warning about "side effects"
  // expected-warning@+1 {{assumption is ignored because it contains (potential) side-effects}}
  __builtin_assume( ({ if (i) ({ stmtexpr_fn(); }); 1; }) );
}
