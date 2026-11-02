// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -pedantic -verify=expected -triple wasm32 -Wno-unused-value -target-feature +reference-types %s
// No error should be emitted.
static __externref_t table[0]; // expected-no-diagnostics
