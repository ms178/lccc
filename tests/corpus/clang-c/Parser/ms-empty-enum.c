// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --ms_extensions --c
// RUN: %clang_cc1 %s -fsyntax-only -Wmicrosoft -verify -fms-extensions

typedef enum tag1 { } A; // expected-warning {{empty enumeration types are a Microsoft extension}}
typedef enum tag2 { } B; // expected-warning {{empty enumeration types are a Microsoft extension}}
typedef enum : unsigned { } C; // expected-warning {{enumeration types with a fixed underlying type are a Microsoft extension}}\
                               // expected-warning {{empty enumeration types are a Microsoft extension}}
