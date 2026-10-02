// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w --c
// RUN: %clang_cc1 -E %s | FileCheck --strict-whitespace %s

#define FOO(x) x
#define BAR(x) x x
#define BAZ(x) [x] [ x] [x ]
[FOO()] [ FOO()] [FOO() ] [BAR()] [ BAR()] [BAR() ] BAZ()
// CHECK: [] [ ] [ ] [ ] [ ] [ ] [] [ ] [ ]
