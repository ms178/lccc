// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -triple mips-linux-gnu  -fsyntax-only -verify %s

void __attribute__((target("arch=mips1")))
foo(void) {}
// expected-error@+3 {{function multiversioning is not supported on the current target}}
// expected-note@-2 {{previous declaration is here}}
void __attribute__((target("arch=mips2")))
foo(void) {}

// expected-error@+2 {{function multiversioning is not supported on the current target}}
void __attribute__((target("default")))
bar(void){}
