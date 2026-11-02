// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 -std=c2x -verify %s
// expected-no-diagnostics

/* WG14 N2670: yes
 * Zeros compare equal
 */
_Static_assert(-1 * 0.0 == 0.0, "");
_Static_assert(!(-1 * 0.0 < 0.0), "");
