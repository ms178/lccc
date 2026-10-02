// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  -w --c
// RUN: %clang_cc1 -fsyntax-only -verify -triple x86_64-pc-win32 %s
// RUN: %clang_cc1 -fsyntax-only -verify -triple x86_64-uefi %s

void __attribute__((ms_abi)) foo(void);
void (*pfoo)(void) = foo;

void __attribute__((sysv_abi)) bar(void);
void (*pbar)(void) = bar; // expected-error{{incompatible function pointer types}}

void (__attribute__((sysv_abi)) *pfoo2)(void) = foo; // expected-error{{incompatible function pointer types}}
