// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
// RUN: %clang_cc1 -triple riscv64 -target-feature +v %s -emit-llvm -o - \
// RUN:            2>&1 | FileCheck %s

#pragma clang riscv intrinsic vvvv
// CHECK:      warning: unexpected argument 'vvvv' to '#pragma riscv'; expected 'vector', 'sifive_vector' or 'andes_vector' [-Wignored-pragmas]

#pragma clang riscv what + 3241
// CHECK:      warning: unexpected argument 'what' to '#pragma riscv'; expected 'intrinsic' [-Wignored-pragmas]
#pragma clang riscv int i = 12;
// CHECK:      warning: unexpected argument 'int' to '#pragma riscv'; expected 'intrinsic' [-Wignored-pragmas]
#pragma clang riscv intrinsic vector bar
// CHECK:     warning: extra tokens at end of '#pragma clang riscv intrinsic' - ignored [-Wignored-pragmas]

#define FOO 0

int main()
{
    return FOO;
}

// Make sure no more warnings
// CHECK-NOT: warning:
