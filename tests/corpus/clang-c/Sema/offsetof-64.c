// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s -triple x86_64-linux-gnu

// PR15216
// Don't crash when taking computing the offset of structs with large arrays.
const unsigned long Size = (1l << 58);

struct Chunk1 {
  char padding[Size]; // expected-warning {{folded to constant}}
  char more_padding[1][Size]; // expected-warning {{folded to constant}}
  char data;
};

unsigned long test1 = __builtin_offsetof(struct Chunk1, data);

struct Chunk2 {
  char padding[Size][Size][Size];  // expected-error {{array is too large}}
  char data;
};

// FIXME: Remove this error when the constant evaluator learns to
// ignore bad types.
int test2 = __builtin_offsetof(struct Chunk2, data);  // expected-error{{initializer element is not a compile-time constant}} 
