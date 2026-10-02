// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple x86_64-pc-linux-gnu -Wmissing-variable-declarations -fsyntax-only -verify %s
// expected-no-diagnostics

register unsigned long current_stack_pointer asm("rsp");
