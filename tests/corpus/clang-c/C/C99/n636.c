// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -verify %s
// RUN: %clang_cc1 -verify=c2x -std=c2x %s

/* WG14 N636: yes
 * remove implicit function declaration
 */

void test(void) {
  frobble(); // expected-error {{call to undeclared function 'frobble'; ISO C99 and later do not support implicit function declarations}} \
                c2x-error {{undeclared identifier 'frobble'}}
}

