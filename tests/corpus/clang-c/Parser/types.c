// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify
// expected-no-diagnostics

// Test the X can be overloaded inside the struct.
typedef int X; 
struct Y { short X; };

// Variable shadows type, PR3872

typedef struct foo { int x; } foo;
void test(void) {
   foo *foo;
   foo->x = 0;
}

