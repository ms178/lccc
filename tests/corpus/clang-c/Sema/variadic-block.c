// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "Sema/variadic-block.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/variadic-block.c" 2



# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 1
# 47 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h"
# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_header_macro.h" 1
# 48 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2



# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___gnuc_va_list.h" 1
# 12 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___gnuc_va_list.h"
typedef __builtin_va_list __gnuc_va_list;
# 52 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_list.h" 1
# 12 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_list.h"
typedef __builtin_va_list va_list;
# 57 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_arg.h" 1
# 62 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg___va_copy.h" 1
# 67 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2




# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/__stdarg_va_copy.h" 1
# 72 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/stdarg.h" 2
# 5 "Sema/variadic-block.c" 2

int main(int argc, char *argv[]) {

    long (^addthem)(const char *, ...) = ^long (const char *format, ...){
        va_list argp;
        const char *p;
        int i;
        char c;
        double d;
        long result = 0;
        __builtin_va_start(argp, format);
        for (p = format; *p; p++) switch (*p) {
            case 'i':
                i = __builtin_va_arg(argp, int);
                result += i;
                break;
            case 'd':
                d = __builtin_va_arg(argp, double);
                result += (int)d;
                break;
            case 'c':
                c = __builtin_va_arg(argp, int);
                result += c;
                break;
        }
        return result;
    };
    long testresult = addthem("ii", 10, 20);
    if (testresult != 30) {
        return 1;
    }
    testresult = addthem("idc", 30, 40.0, 'a');
    if (testresult != (70+'a')) {
        return 1;
    }
    return 0;
}
