// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -verify -triple i386-unknown-unknown %s
// RUN: %clang_cc1 -verify -triple x86_64-unknown-unknown %s
// expected-no-diagnostics

/* WG14 N725: Yes
 * Integer promotion rules
 */
_Static_assert((int)0x80000000U == -2147483648, "");
_Static_assert((unsigned int)-1 == 0xFFFFFFFF, "");

