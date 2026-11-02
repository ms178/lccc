// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -verify %s

/* WG14 N505: Yes
 * Make qualifiers idempotent
 */
const const int i = 12; // expected-warning {{duplicate 'const' declaration specifier}}
typedef const int cint;
const cint j = 12;

