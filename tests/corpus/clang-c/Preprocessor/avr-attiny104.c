// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -dM -triple avr-unknown-unknown -target-cpu attiny104 /dev/null | FileCheck -match-full-lines %s

// CHECK: #define AVR 1
// CHECK: #define __AVR 1
// CHECK: #define __AVR_ARCH__ 100
// CHECK: #define __AVR_ATtiny104__ 1
// CHECK-NOT: #define __AVR_HAVE_MUL__ 1
// CHECK: #define __AVR__ 1
// CHECK: #define __ELF__ 1
