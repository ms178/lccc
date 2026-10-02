// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fexperimental-strict-floating-point \
// RUN: -triple i386-pc-windows -target-cpu pentium4 -target-feature -sse \
// RUN: -emit-llvm -ffp-eval-method=source  -o - -verify=warn %s
//
// RUN: %clang_cc1 -fexperimental-strict-floating-point \
// RUN: -triple i386-pc-windows -target-cpu pentium4 \
// RUN: -emit-llvm -ffp-eval-method=source  -o - -verify=no-warn %s

// no-warn-no-diagnostics

float add1(float a, float b, float c) {
  return a + b + c;
} // warn-warning{{setting the floating point evaluation method to `source` on a target without SSE is not supported}}

float add2(float a, float b, float c) {
#pragma clang fp eval_method(source)
  return a + b + c;
} // warn-warning{{setting the floating point evaluation method to `source` on a target without SSE is not supported}}
