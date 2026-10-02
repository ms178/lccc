// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple powerpc-linux-gnu -fsyntax-only -verify %s

// Test that we trigger an error at parse time if using keyword funcref
// while not using a wasm triple.
typedef void (*__funcref funcref_t)();     // expected-error {{invalid use of '__funcref' keyword outside the WebAssembly triple}}
typedef int (*__funcref fn_funcref_t)(int);// expected-error {{invalid use of '__funcref' keyword outside the WebAssembly triple}}
typedef int (*fn_t)(int);

static fn_funcref_t nullFuncref = 0;
