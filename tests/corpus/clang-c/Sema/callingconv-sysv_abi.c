// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -verify -triple x86_64-pc-linux-gnu %s

void __attribute__((ms_abi)) foo(void);
void (*pfoo)(void) = foo; // expected-error{{incompatible function pointer types}}

void __attribute__((sysv_abi)) bar(void);
void (*pbar)(void) = bar;

void (__attribute__((ms_abi)) *pbar2)(void) = bar; // expected-error{{incompatible function pointer types}}
