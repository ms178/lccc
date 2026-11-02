// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c23
// RUN: %clang_cc1 -verify -std=c2y %s
// RUN: %clang_cc1 -verify -std=c23 %s

/* WG14 N3481: Yes
 * Slay Some Earthly Demons XVI
 *
 * It was previously UB to use a non-array lvalue with an incomplete type in a
 * context which required the value of the object. Clang has always diagnosed
 * this as an error, except when the incomplete type is void. Then we allow the
 * dereference, but not for a value computation.
 */

struct f *p;  // expected-note {{forward declaration of 'struct f'}}
void g(void) {
  (void)*p; // expected-error {{incomplete type 'struct f' where a complete type is required}}
}

void h(void *ptr) {
  (void)*ptr; // expected-warning {{ISO C does not allow indirection on operand of type 'void *'}}
  (*ptr)++;   /* expected-warning {{ISO C does not allow indirection on operand of type 'void *'}}
                 expected-error {{cannot increment value of type 'void'}}
               */
}
