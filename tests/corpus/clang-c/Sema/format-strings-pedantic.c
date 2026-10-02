// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 -fsyntax-only -verify -Wno-format -Wformat-pedantic %s
// RUN: %clang_cc1 -xobjective-c -fblocks -fsyntax-only -verify -Wno-format -Wformat-pedantic %s
// RUN: %clang_cc1 -xc++ -fsyntax-only -verify -Wno-format -Wformat-pedantic %s
// RUN: %clang_cc1 -std=c23 -fsyntax-only -verify -Wno-format -Wformat-pedantic %s

__attribute__((format(printf, 1, 2)))
int printf(const char *restrict, ...);

int main(void) {
  printf("%p", (int *)0); // expected-warning {{format specifies type 'void *' but the argument has type 'int *'}}
  printf("%p", (void *)0);

#ifdef __OBJC__
  printf("%p", ^{}); // expected-warning {{format specifies type 'void *' but the argument has type 'void (^)(void)'}}
  printf("%p", (id)0); // expected-warning {{format specifies type 'void *' but the argument has type 'id'}}
#endif

#if !__is_identifier(nullptr)
  printf("%p", nullptr);
#endif
}
