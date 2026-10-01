// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

// PR3592
static void* malloc(int);
static void* malloc(int size) {
  return ((void*)0); /*do not use heap in this file*/
}

void *calloc(int, int, int); // expected-warning{{incompatible redeclaration of library function 'calloc'}} \
// expected-note{{'calloc' is a builtin with type 'void *}}

void f1(void) { 
  calloc(0, 0, 0);
}

void f2(void) {
  int index = 1;
}

static int index;

int f3(void) {
  return index << 2;
}

typedef int rindex;
