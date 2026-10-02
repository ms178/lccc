// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple s390x-linux-gnu -fsyntax-only -verify %s

// expected-warning@+1 {{unknown attribute 'function_return' ignored}}
__attribute__((function_return("keep"))) void x(void) {}

// expected-warning@+1 {{unknown attribute 'function_return' ignored}}
__attribute__((function_return("thunk"))) void y(void) {}

// expected-warning@+1 {{unknown attribute 'function_return' ignored}}
__attribute__((function_return("thunk-inline"))) void z(void) {}

// expected-warning@+1 {{unknown attribute 'function_return' ignored}}
__attribute__((function_return("thunk-extern"))) void w(void) {}

// expected-warning@+1 {{unknown attribute 'function_return' ignored}}
__attribute__((function_return("invalid"))) void v(void) {}
