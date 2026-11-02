// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
/* RUN: %clang_cc1 -E %s | grep 'a 3'
 * RUN: %clang_cc1 -E %s | grep 'b 16'
 * RUN: %clang_cc1 -E -P %s | grep 'a 3'
 * RUN: %clang_cc1 -E -P %s | grep 'b 16'
 * RUN: %clang_cc1 -E %s | not grep '# 0 '
 * RUN: %clang_cc1 -E -P %s | count 2
 * PR1848 PR3437 PR7360
*/

#define t(x) x

t(a
3)

t(b
__LINE__)

