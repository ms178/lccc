// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -Dtest=FOO --c
// RUN: %clang_cc1 -E -Dtest=FOO -imacros %S/pr2086.h %s | grep 'HERE: test'

// This should not be expanded into FOO because pr2086.h undefs 'test'.
HERE: test
