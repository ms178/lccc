// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23
// RUN: %clang_cc1 -std=c23 -Wno-underlying-atomic-qualifier-ignored -ast-dump %s | FileCheck %s

// The underlying type is the unqualified, non-atomic version of the type
// specified.
enum const_enum : const short { ConstE };
// CHECK: EnumDecl {{.*}} const_enum 'short'

// These were previously being diagnosed as invalid underlying types. They
// are valid; the _Atomic is stripped from the underlying type.
enum atomic_enum1 : _Atomic(int) { AtomicE1 };
// CHECK: EnumDecl {{.*}} atomic_enum1 'int'
enum atomic_enum2 : _Atomic long long { AtomicE2 };
// CHECK: EnumDecl {{.*}} atomic_enum2 'long long'
