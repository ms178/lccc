// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
/* RUN: %clang_cc1 -E -verify %s
 */

/* expected-error@+1 {{macro name missing}} */
#ifdef
#endif

/* expected-error@+1 {{macro name must be an identifier}} */
#ifdef !
#endif

/* expected-error@+1 {{macro name missing}} */
#if defined
#endif

/* PR1936 */
/* expected-error@+2 {{unterminated function-like macro invocation}} expected-error@+2 {{expected value in expression}} expected-note@+1 {{macro 'f' defined here}} */
#define f(x) x
#if f(2
#endif

/* expected-error@+3 {{macro name missing}} */
/* expected-warning@+2 {{use of a '#elifdef' directive is a C23 extension}} */
#ifdef FOO
#elifdef
#endif

/* expected-error@+3 {{macro name must be an identifier}} */
/* expected-warning@+2 {{use of a '#elifdef' directive is a C23 extension}} */
#ifdef FOO
#elifdef !
#endif

int x;
