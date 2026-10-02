// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -E -I%S %s | grep BODY_OF_FILE | wc -l | grep 1

// This #import should have no effect, as we're importing the current file.
#import <import_self.c>

BODY_OF_FILE

