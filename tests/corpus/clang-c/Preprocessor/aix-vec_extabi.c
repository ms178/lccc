// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang -target powerpc-ibm-aix-xcoff -mcpu=pwr8 -E -dM -maltivec -mabi=vec-extabi %s -o - 2>&1 \
// RUN:   | FileCheck %s -check-prefix=EXTABI
// RUN: %clang  -target powerpc64-ibm-aix-xcoff -mcpu=pwr8 -E -dM -maltivec -mabi=vec-extabi %s -o - 2>&1 \
// RUN:   | FileCheck %s -check-prefix=EXTABI
// RUN: %clang  -target powerpc-ibm-aix-xcoff -mcpu=pwr8 -E -dM -maltivec -mabi=vec-default %s 2>&1 \
// RUN:   | FileCheck %s -check-prefix=DFLTABI
// RUN: %clang -target powerpc64-ibm-aix-xcoff -mcpu=pwr8 -E -dM -maltivec -mabi=vec-default %s 2>&1 \
// RUN:   | FileCheck %s -check-prefix=DFLTABI


// EXTABI:      #define __EXTABI__
// DFLTABI-NOT: #define __EXTABI__
