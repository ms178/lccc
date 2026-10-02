// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "Sema/format-strings-signedness-fixit.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/format-strings-signedness-fixit.c" 2





# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h" 1
# 25 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h"
# 1 "/usr/include/limits.h" 1 3 4
# 26 "/usr/include/limits.h" 3 4
# 1 "/usr/include/features.h" 1 3 4
# 345 "/usr/include/features.h" 3 4
# 1 "/usr/include/stdc-predef.h" 1 3 4
# 346 "/usr/include/features.h" 2 3 4
# 375 "/usr/include/features.h" 3 4
# 1 "/usr/include/sys/cdefs.h" 1 3 4
# 392 "/usr/include/sys/cdefs.h" 3 4
# 1 "/usr/include/bits/wordsize.h" 1 3 4
# 393 "/usr/include/sys/cdefs.h" 2 3 4
# 376 "/usr/include/features.h" 2 3 4
# 399 "/usr/include/features.h" 3 4
# 1 "/usr/include/gnu/stubs.h" 1 3 4
# 10 "/usr/include/gnu/stubs.h" 3 4
# 1 "/usr/include/gnu/stubs-64.h" 1 3 4
# 11 "/usr/include/gnu/stubs.h" 2 3 4
# 400 "/usr/include/features.h" 2 3 4
# 27 "/usr/include/limits.h" 2 3 4
# 144 "/usr/include/limits.h" 3 4
# 1 "/usr/include/bits/posix1_lim.h" 1 3 4
# 160 "/usr/include/bits/posix1_lim.h" 3 4
# 1 "/usr/include/bits/local_lim.h" 1 3 4
# 38 "/usr/include/bits/local_lim.h" 3 4
# 1 "/usr/include/linux/limits.h" 1 3 4
# 39 "/usr/include/bits/local_lim.h" 2 3 4
# 161 "/usr/include/bits/posix1_lim.h" 2 3 4
# 145 "/usr/include/limits.h" 2 3 4



# 1 "/usr/include/bits/posix2_lim.h" 1 3 4
# 149 "/usr/include/limits.h" 2 3 4
# 26 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/limits.h" 2
# 7 "Sema/format-strings-signedness-fixit.c" 2

int printf(const char *restrict format, ...);

void test_printf_int(int x)
{
    printf("%u", x);
}

void test_printf_unsigned(unsigned x)
{
    printf("%d", x);
}

void test_printf_long(long x)
{
    printf("%lu", x);
}

void test_printf_unsigned_long(unsigned long x)
{
    printf("%ld", x);
}

void test_printf_long_long(long long x)
{
    printf("%llu", x);
}

void test_printf_unsigned_long_long(unsigned long long x)
{
    printf("%lld", x);
}

enum enum_int {
    minus_1 = -1
};

void test_printf_enum_int(enum enum_int x)
{
    printf("%u", x);
}

enum enum_unsigned {
    zero = 0
};

void test_printf_enum_unsigned(enum enum_unsigned x)
{
    printf("%d", x);
}

enum enum_long {
    minus_one = -1,
    int_val = 2147483647,
    unsigned_val = (unsigned)(-2147483647 -1)
};

void test_printf_enum_long(enum enum_long x)
{
    printf("%lu", x);
}

enum enum_unsigned_long {
    uint_max_plus = (unsigned long)(2147483647 *2U +1U)+1,
};

void test_printf_enum_unsigned_long(enum enum_unsigned_long x)
{
    printf("%ld", x);
}
