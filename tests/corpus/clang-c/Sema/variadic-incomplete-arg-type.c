// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 %s -fsyntax-only -verify 

typedef struct __CFError * CFErrorRef; // expected-note {{forward declaration of 'struct __CFError'}}

void junk(int, ...);

int main(void)
{
 CFErrorRef error;
 junk(1, *error, (void)0); // expected-error {{argument type 'struct __CFError' is incomplete}} \
                           // expected-error {{argument type 'void' is incomplete}}
}
