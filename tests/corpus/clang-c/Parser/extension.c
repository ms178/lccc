// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c89
/* RUN: %clang_cc1 %s -fsyntax-only -pedantic -verify -std=c89
 */

/* Top level extension marker. */

__extension__ typedef struct
{
    long long int quot; 
    long long int rem; 
} lldiv_t;


/* Decl/expr __extension__ marker. */
void bar(void) {
  __extension__ int i;
  int j;
  __extension__ (j = 10LL);
  __extension__ j = 10LL; /* expected-warning {{'long long' is an extension}} */
}

