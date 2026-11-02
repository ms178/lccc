// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -fsyntax-only -Wdocumentation %s
// The run line does not have '-verify' because we were crashing while printing
// the diagnostic.

// This file has DOS-style line endings (CR LF).  Please don't change it to
// Unix-style LF!

// PR14591.  Check that we don't crash on this.
/**
 * @param abc
 */
void nocrash1(int qwerty);

