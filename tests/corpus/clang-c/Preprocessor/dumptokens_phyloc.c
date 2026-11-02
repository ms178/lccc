// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -dump-tokens %s 2>&1 | grep "Spelling=.*dumptokens_phyloc.c:3:20"

#define TESTPHYLOC 10

TESTPHYLOC
