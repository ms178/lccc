// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -triple=i686-apple-darwin9 -verify

struct foo {
  int big[128];
};
struct bar {
  char c[3];
};

struct bar smallThing;
struct foo bigThing;
_Atomic(struct foo) bigAtomic;

void structAtomicStore(void) {
  struct foo f = {0};
  __c11_atomic_store(&bigAtomic, f, 5); // expected-error {{atomic store requires runtime support that is not available for this target}}

  struct bar b = {0};
  __atomic_store(&smallThing, &b, 5);

  __atomic_store(&bigThing, &f, 5);
}

void structAtomicLoad(void) {
  struct foo f = __c11_atomic_load(&bigAtomic, 5); // expected-error {{atomic load requires runtime support that is not available for this target}}
  struct bar b;
  __atomic_load(&smallThing, &b, 5);

  __atomic_load(&bigThing, &f, 5);
}
