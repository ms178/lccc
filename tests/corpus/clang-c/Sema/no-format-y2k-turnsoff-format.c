// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -verify -fsyntax-only -Wformat -Wno-format-y2k %s

void foo(const char *, ...) __attribute__((__format__ (__printf__, 1, 2)));

void bar(unsigned int a) {
        foo("%s", a); // expected-warning {{format specifies type 'char *' but the argument has type 'unsigned int'}}
}

