// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: printf -- "-DX=A\\\\\nTHIS_SHOULD_NOT_EXIST_IN_THE_OUTPUT\n" | xargs %clang -E %s | FileCheck -strict-whitespace %s

// Per GCC -D semantics, \n and anything that follows is ignored.

// CHECK: {{^START A END$}}
START X END
