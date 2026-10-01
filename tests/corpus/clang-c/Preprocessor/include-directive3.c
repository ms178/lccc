// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -include %S/file_to_include.h -E %s -fno-caret-diagnostics 2>&1 >/dev/null | grep 'file successfully included' | count 1
// PR3464

