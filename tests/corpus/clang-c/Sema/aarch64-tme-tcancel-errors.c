// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -triple aarch64 -target-feature +tme -verify %s
void t_cancel_const(unsigned short u) {
  __builtin_arm_tcancel(u); // expected-error{{argument to '__builtin_arm_tcancel' must be a constant integer}}
}

// RUN: %clang_cc1 -triple aarch64 -target-feature +tme -verify %s
void t_cancel_range(void) {
  __builtin_arm_tcancel(0x12345u); // expected-error{{argument value 74565 is outside the valid range [0, 65535]}}
}
