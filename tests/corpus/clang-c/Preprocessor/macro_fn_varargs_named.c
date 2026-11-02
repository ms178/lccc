// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w -w -w --c
// RUN: %clang_cc1 -E %s | FileCheck %s --match-full-lines --strict-whitespace --check-prefix CHECK-1
// CHECK-1:a: x
// RUN: %clang_cc1 -E %s | FileCheck %s --match-full-lines --strict-whitespace --check-prefix CHECK-2
// CHECK-2:b: x y, z,h
// RUN: %clang_cc1 -E %s | FileCheck %s --match-full-lines --strict-whitespace --check-prefix CHECK-3
// CHECK-3:c: foo(x)

#define A(b, c...) b c
a: A(x)
b: A(x, y, z,h)

#define B(b, c...) foo(b, ## c)
c: B(x)
