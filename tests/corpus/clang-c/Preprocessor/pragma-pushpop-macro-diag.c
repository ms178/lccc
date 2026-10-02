// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --ms_extensions --c
// RUN: %clang_cc1 -fms-extensions %s -fsyntax-only -verify

#pragma push_macro("") // expected-warning {{'#pragma push_macro' expected a non-empty string}}
#pragma pop_macro("") // expected-warning {{'#pragma pop_macro' expected a non-empty string}}
