// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -triple aarch64-unknown-linux-gnu -aux-triple aarch64 -target-feature +sve -fopenmp-is-target-device -fopenmp -verify -fsyntax-only %s

static __inline__ __attribute__((__clang_arm_builtin_alias(__builtin_sve_svabd_n_f64_m))) // expected-no-diagnostics
void
nop(void);
