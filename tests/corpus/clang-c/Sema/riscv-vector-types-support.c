// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -triple riscv64 -fsyntax-only -verify

typedef __attribute__((riscv_rvv_vector_bits(256))) void norvvflag; // expected-error{{'riscv_rvv_vector_bits' attribute is not supported on targets missing 'zve32x'; specify an appropriate -march= or -mcpu=}}
