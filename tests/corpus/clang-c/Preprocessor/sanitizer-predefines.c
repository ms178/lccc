// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c --target linux_aarch64
// RUN: %clang_cc1 -E -dM -triple aarch64-unknown-linux -fsanitize=address %s | FileCheck %s --check-prefix=ASAN
// RUN: %clang_cc1 -E -dM -triple aarch64-unknown-linux -fsanitize=kernel-address %s | FileCheck %s --check-prefix=ASAN
// ASAN: #define __SANITIZE_ADDRESS__ 1

// RUN: %clang_cc1 -E -dM -triple aarch64-unknown-linux -fsanitize=hwaddress %s | FileCheck %s --check-prefix=HWASAN
// RUN: %clang_cc1 -E -dM -triple aarch64-unknown-linux -fsanitize=kernel-hwaddress %s | FileCheck %s --check-prefix=HWASAN
// HWASAN: #define __SANITIZE_HWADDRESS__ 1

// RUN: %clang_cc1 -E -dM -triple aarch64-unknown-linux -fsanitize=thread %s | FileCheck %s --check-prefix=TSAN
// TSAN: #define __SANITIZE_THREAD__ 1
