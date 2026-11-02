// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 < %s -E -verify -triple i686-pc-linux-gnu
// expected-no-diagnostics

#if (('1234' >> 24) != '1')
#error Bad multichar constant calculation!
#endif
