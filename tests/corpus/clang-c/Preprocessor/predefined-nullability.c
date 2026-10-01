// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --check-prefix=CHECK-DARWIN --c
// RUN: %clang_cc1 %s -E -dM -triple i386-apple-darwin10 -o - | FileCheck %s --check-prefix=CHECK-DARWIN

// RUN: %clang_cc1 %s -E -dM -triple x86_64-unknown-linux -o - | FileCheck %s --check-prefix=CHECK-NONDARWIN


// CHECK-DARWIN: #define __nonnull _Nonnull
// CHECK-DARWIN: #define __null_unspecified _Null_unspecified
// CHECK-DARWIN: #define __nullable _Nullable

// CHECK-NONDARWIN-NOT: __nonnull
// CHECK-NONDARWIN: #define __clang__
// CHECK-NONDARWIN-NOT: __nonnull
