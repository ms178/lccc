// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -triple aarch64 -verify -fsyntax-only %s

__attribute__((target("branch-protection=foo"))) // expected-error {{invalid or misplaced branch protection specification 'foo'}}
void
badvalue0(void) {}

__attribute__((target("branch-protection=+bti"))) // expected-error {{invalid or misplaced branch protection specification '<empty>'}}
void
badvalue1(void) {}

__attribute__((target("branch-protection=bti+"))) // expected-error {{invalid or misplaced branch protection specification '<empty>'}}
void
badvalue2(void) {}

__attribute__((target("branch-protection=pac-ret+bkey"))) // expected-error {{invalid or misplaced branch protection specification 'bkey'}}
void
badvalue3(void) {}

__attribute__((target("branch-protection=bti+leaf"))) // expected-error {{invalid or misplaced branch protection specification 'leaf'}}
void
badoption0(void) {}

__attribute__((target("branch-protection=bti+leaf+pac-ret"))) // expected-error {{invalid or misplaced branch protection specification 'leaf'}}
void
badorder0(void) {}

__attribute__((target("branch-protection=pac-ret+bti+leaf"))) // expected-error {{invalid or misplaced branch protection specification 'leaf'}}
void
badorder1(void) {}
