// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang %s -dM -E -target x86_64-windows | FileCheck %s --check-prefix=NO
// RUN: %clang %s -dM -E -target x86_64 -fno-asynchronous-unwind-tables | FileCheck %s --check-prefix=NO
// RUN: %clang %s -dM -E -target x86_64 -fno-dwarf2-cfi-asm | FileCheck %s --check-prefix=NO

// RUN: %clang %s -dM -E -target x86_64 | FileCheck %s
// RUN: %clang %s -dM -E -target x86_64 -funwind-tables -fno-asynchronous-unwind-tables -g | FileCheck %s
// RUN: %clang %s -dM -E -target aarch64-apple-darwin | FileCheck %s
// RUN: %clang %s -dM -E -target x86_64 -fno-asynchronous-unwind-tables -g | FileCheck %s
// RUN: %clang %s -dM -E -target x86_64 -fno-asynchronous-unwind-tables -fexceptions | FileCheck %s
// RUN: %clang %s -dM -E -target x86_64-windows -fdwarf2-cfi-asm | FileCheck %s

// NO-NOT: #define __GCC_HAVE_DWARF2_CFI_ASM
// CHECK: #define __GCC_HAVE_DWARF2_CFI_ASM 1
