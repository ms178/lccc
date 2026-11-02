// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c99: --c89
// RUN: %clang_cc1 -fsyntax-only -verify -std=c99 -Wc11-extensions %s
// RUN: %clang_cc1 -fsyntax-only -verify -std=c89 -Wc11-extensions %s

int incomplete[]; // expected-warning {{tentative array definition assumed to have one element}}
int complete[6];

int test_comparison_between_incomplete_and_complete_pointer(void) {
  return (&incomplete < &complete) &&  // expected-warning {{pointer comparisons before C11 need to be between two complete or two incomplete types; 'int (*)[]' is incomplete and 'int (*)[6]' is complete}}
         (&incomplete <= &complete) && // expected-warning {{pointer comparisons before C11 need to be between two complete or two incomplete types; 'int (*)[]' is incomplete and 'int (*)[6]' is complete}}
         (&incomplete > &complete) &&  // expected-warning {{pointer comparisons before C11 need to be between two complete or two incomplete types; 'int (*)[]' is incomplete and 'int (*)[6]' is complete}}
         (&incomplete >= &complete) && // expected-warning {{pointer comparisons before C11 need to be between two complete or two incomplete types; 'int (*)[]' is incomplete and 'int (*)[6]' is complete}}
         (&incomplete == &complete) &&
         (&incomplete != &complete);
}
