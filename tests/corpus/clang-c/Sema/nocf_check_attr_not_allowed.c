// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple powerpc-unknown-linux-gnu -fsyntax-only -verify -fcf-protection=branch %s
// RUN: %clang_cc1 -triple arm-unknown-linux-gnu -fsyntax-only -verify -fcf-protection=branch %s
// RUN: %clang_cc1 -triple arm-unknown-linux-gnu -fsyntax-only -verify %s

void __attribute__((nocf_check)) foo(void); // expected-warning-re{{{{((unknown attribute 'nocf_check' ignored)|('nocf_check' attribute ignored; use -fcf-protection to enable the attribute))}}}}
