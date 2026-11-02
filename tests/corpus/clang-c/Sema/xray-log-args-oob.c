// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c11
// RUN: %clang_cc1 %s -verify -fsyntax-only -std=c11
void foo(int) __attribute__((xray_log_args(1)));
struct __attribute__((xray_log_args(1))) a { int x; }; // expected-warning {{'xray_log_args' attribute only applies to functions and Objective-C methods}}

void fop(void) __attribute__((xray_log_args(1))); // expected-error {{'xray_log_args' attribute parameter 1 is out of bounds}}

void foq(void) __attribute__((xray_log_args(-1))); // expected-error {{'xray_log_args' attribute parameter 1 is out of bounds}}

void fos(void) __attribute__((xray_log_args(0))); // expected-error {{'xray_log_args' attribute parameter 1 is out of bounds}}
