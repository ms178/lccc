// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c89: --c89: --c89: --c89: --c89: --c99: --c99: --c99: --c89: --c99
/* RUN: %clang_cc1 -verify=off -std=c89 %s
 * RUN: %clang_cc1 -verify=off -Wall -std=c89 %s
 * RUN: %clang_cc1 -verify -pedantic -std=c89 %s
 * RUN: %clang_cc1 -verify -Wvla-extension -std=c89 %s
 * RUN: %clang_cc1 -verify=off -Wvla-cxx-extension -std=c89 %s
 * RUN: %clang_cc1 -verify=off -pedantic -std=c99 %s
 * RUN: %clang_cc1 -verify=off -Wall -std=c99 %s
 * RUN: %clang_cc1 -verify=off -std=c99 -Wvla-extension %s
 * The next run line still issues the extension warning because VLAs are an
 * extension in C89, but the line after it will issue the congratulatory
 * diagnostic.
 * RUN: %clang_cc1 -verify -Wvla -std=c89 %s
 * RUN: %clang_cc1 -verify=wvla -Wvla -std=c99 %s
 */

/* off-no-diagnostics */

void func(int n) {
  int array[n]; /* expected-warning {{variable length arrays are a C99 feature}}
                   wvla-warning {{variable length array used}}
                 */
  (void)array;
}

