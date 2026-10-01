// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -E -CC -pedantic -verify
// expected-no-diagnostics

#if 1 /*bar */

#endif /*foo*/

#if /*foo*/ defined /*foo*/ FOO /*foo*/
#if /*foo*/ defined /*foo*/ ( /*foo*/ FOO /*foo*/ ) /*foo*/
#endif
#endif

