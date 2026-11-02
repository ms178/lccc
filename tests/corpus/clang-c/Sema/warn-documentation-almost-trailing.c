// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wdocumentation -verify %s
// RUN: %clang_cc1 -fsyntax-only -Wdocumentation -fdiagnostics-parseable-fixits %s 2>&1 | FileCheck %s
// RUN: cp %s %t
// RUN: %clang_cc1 -Wdocumentation -fixit %t
// RUN: %clang_cc1 -fsyntax-only -Wdocumentation -Werror %t

struct a {
  int x; //< comment // expected-warning {{not a Doxygen trailing comment}}
  int y; /*< comment */ // expected-warning {{not a Doxygen trailing comment}}
};

// CHECK: fix-it:"{{.*}}":{[[@LINE-4]]:10-[[@LINE-4]]:13}:"///<"
// CHECK: fix-it:"{{.*}}":{[[@LINE-4]]:10-[[@LINE-4]]:13}:"/**<"
