// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple i686-apple-darwin9 %s -fsyntax-only -verify
// expected-no-diagnostics

// Pragma pack handling with tag declarations

struct X;

#pragma pack(2)
struct X { int x; };
struct Y;
#pragma pack()

struct Y { int y; };

extern int check[__alignof(struct X) == 2 ? 1 : -1];
extern int check[__alignof(struct Y) == 4 ? 1 : -1];

