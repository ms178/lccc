// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -verify -std=c2x %s

/* WG14 N2930: yes
 * Consider renaming remove_quals
 */

int remove_quals;
int typeof_unqual; // expected-error {{expected '(' after 'typeof_unqual'}}
typeof_unqual(remove_quals) val;
