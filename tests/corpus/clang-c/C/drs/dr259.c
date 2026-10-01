// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c89: --c99: --c11: --c17: --c23
/* RUN: %clang_cc1 -std=c89 -E -verify %s | FileCheck %s
   RUN: %clang_cc1 -std=c99 -E -verify %s | FileCheck %s
   RUN: %clang_cc1 -std=c11 -E -verify %s | FileCheck %s
   RUN: %clang_cc1 -std=c17 -E -verify %s | FileCheck %s
   RUN: %clang_cc1 -std=c2x -E -verify %s | FileCheck %s
 */

/* expected-no-diagnostics */

/* WG14 DR259: yes
 * Macro invocations with no arguments
 */
#define m0() replacement
#define m1(x) begin x end

m0() m1()

/*
CHECK: replacement begin end
*/

