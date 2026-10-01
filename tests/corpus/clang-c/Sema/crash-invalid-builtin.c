// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple=x86_64-apple-darwin -fsyntax-only -verify %s
// PR23086

__builtin_isinf(...); // expected-error {{type specifier missing, defaults to 'int'}} expected-error {{ISO C requires a named parameter before '...'}} // expected-error {{cannot redeclare builtin function '__builtin_isinf'}} // expected-note {{'__builtin_isinf' is a builtin with type 'int ()'}}
