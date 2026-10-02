// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c23: --c17 -D_CLANG_DISABLE_CRT_DEPRECATION_WARNINGS: --c23 -D_CLANG_DISABLE_CRT_DEPRECATION_WARNINGS: --c17
# 1 "Sema/c2x-noreturn.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 444 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/c2x-noreturn.c" 2
# 26 "Sema/c2x-noreturn.c"
[[noreturn(12)]] void func4(void);
[[noreturn]] int not_a_func;
void func5(void) [[noreturn]];


_Noreturn void func1(void);
[[noreturn]] void func2(void);



[[_Noreturn]] void func3(void);


# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdnoreturn.h" 1
# 40 "Sema/c2x-noreturn.c" 2

[[_Noreturn]] void func6(void);

void func7 [[_Noreturn]] (void);

_Noreturn void func8(void);


void _Noreturn func9(void);


[[_Noreturn]] void func10(void);
# 65 "Sema/c2x-noreturn.c"
[[_Noreturn]] void func11(void);
