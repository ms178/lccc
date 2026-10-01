// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -fsyntax-only -verify -triple i686-linux-pc -target-feature +sse2 %s
// RUN: %clang_cc1 -fsyntax-only -verify -triple x86_64-linux-pc %s
// RUN: %clang_cc1 -fsyntax-only -verify -triple spir-unknown-unknown %s
// RUN: %clang_cc1 -fsyntax-only -verify -triple armv7a-linux-gnu %s
// RUN: %clang_cc1 -fsyntax-only -verify -triple aarch64-linux-gnu %s
// RUN: %clang_cc1 -fsyntax-only -verify -triple riscv32 %s
// RUN: %clang_cc1 -fsyntax-only -verify -triple riscv64 %s

void a(const char *a, ...) __attribute__((format(printf, 1, 2)));    // no-error

void b(char *a, _Float16 b) __attribute__((format(printf, 1, 2))); // expected-warning {{GCC requires a function with the 'format' attribute to be variadic}}

void call_no_default_promotion(void) {
  a("%f", (_Float16)1.0); // expected-warning{{format specifies type 'double' but the argument has type '_Float16'}}
  b("%f", (_Float16)1.0); // expected-warning{{format specifies type 'double' but the argument has type '_Float16'}}
}
