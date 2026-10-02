// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23: --c17: --c23
// RUN: %clang_cc1 -verify -std=c23 %s
// RUN: %clang_cc1 -verify=pedantic -std=c17 -pedantic %s
// RUN: %clang_cc1 -verify=compat -std=c23 -Wpre-c23-compat %s

// expected-no-diagnostics

/* WG14 N2549: Clang 9
 * Binary literals
 */

int i = 0b01; /* pedantic-warning {{binary integer literals are a C23 extension}}
                 compat-warning {{binary integer literals are incompatible with C standards before C23}}
               */

