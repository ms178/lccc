// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  -w -w -w --c
// RUN: %clang_cc1 -E -C %s | FileCheck -strict-whitespace %s
// RUN: %clang_cc1 -E -C -fminimize-whitespace %s | FileCheck -strict-whitespace %s

// foo
// CHECK: // foo

/* bar */
// CHECK: /* bar */

#if FOO
#endif
/* baz */
// CHECK: /* baz */

_Pragma("unknown") // after unknown pragma
// CHECK: #pragma unknown
// CHECK-NEXT: #
// CHECK-NEXT: // after unknown pragma

_Pragma("comment(\"abc\")") // after known pragma
// CHECK: #pragma comment("abc")
// CHECK-NEXT: #
// CHECK-NEXT: // after known pragma
