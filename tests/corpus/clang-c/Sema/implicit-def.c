// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c89: --c99
/* RUN: %clang_cc1 -fsyntax-only %s -std=c89
 * RUN: not %clang_cc1 -fsyntax-only %s -std=c99 -pedantic-errors
 */

int A(void) {
  return X();
}

