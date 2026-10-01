// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --ms_extensions --c
// RUN: %clang_cc1 -triple i686-windows-msvc -fms-extensions -fsyntax-only -verify %s

struct {} __cdecl s; // expected-warning {{'__cdecl' only applies to function types; type here is 'struct}}
