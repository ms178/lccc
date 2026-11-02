// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify %s

struct spinlock_t {
  int lock;
} audit_skb_queue;

void fn1(void) {
  audit_skb_queue = (lock); // expected-error {{use of undeclared identifier 'lock'}}
}

void fn2(void) {
  audit_skb_queue + (lock); // expected-error {{use of undeclared identifier 'lock'}}
}
