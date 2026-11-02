// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c11
// RUN: %clang_cc1 -std=c11 %s -fsyntax-only -verify
// REQUIRES: asserts

struct dispatch_object_s;
void _dispatch_queue_get_head(struct dispatch_object_s *volatile dq_items_head) {
  (_Atomic __typeof__(dq_items_head) *)0; // expected-warning{{expression result unused}}
}
void g(void) {
  (_Atomic __typeof__(struct dispatch_object_s *volatile) *)0; // expected-warning{{expression result unused}}
}
