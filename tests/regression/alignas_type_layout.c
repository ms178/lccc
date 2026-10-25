/*
 * `_Alignas(type-name)` on struct members whose alignment the parser cannot
 * know (typedefs, enums, records built from them), and the strictest-wins
 * merge of several alignment sources (C11 6.7.5p6). The parser used to guess
 * (typedef -> pointer size, enum -> 4) and the guess decided the layout:
 * `struct { char a; _Alignas(uint16_t) char b; }` was 16 bytes, not 4.
 * Expected values hold for GCC 16.2 and Clang 23.1.4 on x86-64, i686 and
 * AArch64; every fact is asserted statically and checked at run time.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

typedef char C;
typedef uint16_t U16;
typedef struct { char x[3]; } S3;
typedef int __attribute__((aligned(16))) I16;
enum Mix { MA, MB = 1LL << 40 };
enum __attribute__((packed)) P { PA };
struct W { char c; enum Mix m; };

struct A1 { char a; _Alignas(C) char b; };
struct A2 { char a; _Alignas(U16) char b; };
struct A3 { char a; _Alignas(S3) char b; };
struct A4 { char a; _Alignas(enum Mix) char b; };
struct A5 { char a; _Alignas(enum P) char b; };
struct A6 { char a; _Alignas(struct W) char b; };
struct A7 { char a; _Alignas(I16) char b; };
struct A8 { char a; _Alignas(U16) char b __attribute__((aligned(8))); };
struct A9 { char a; _Alignas(16) char b __attribute__((aligned(4))); };

#define LL_ALIGN _Alignof(unsigned long long)
#define CHECKS(X)                                                              \
    X(offsetof(struct A1, b) == 1 && sizeof(struct A1) == 2)                   \
    X(offsetof(struct A2, b) == 2 && sizeof(struct A2) == 4)                   \
    X(offsetof(struct A3, b) == 1 && sizeof(struct A3) == 2)                   \
    X(offsetof(struct A4, b) == LL_ALIGN && sizeof(struct A4) == 2 * LL_ALIGN) \
    X(offsetof(struct A5, b) == 1 && sizeof(struct A5) == 2)                   \
    X(_Alignof(struct W) == LL_ALIGN && offsetof(struct A6, b) == LL_ALIGN)    \
    X(offsetof(struct A7, b) == 16 && sizeof(struct A7) == 32)                 \
    X(offsetof(struct A8, b) == 8 && sizeof(struct A8) == 16)                  \
    X(offsetof(struct A9, b) == 16 && sizeof(struct A9) == 32)

#define STATIC_CHECK(cond) _Static_assert(cond, #cond);
CHECKS(STATIC_CHECK)

int main(void)
{
    int fails = 0;
#define RUNTIME_CHECK(cond) fails += !(cond);
    CHECKS(RUNTIME_CHECK)
    puts(fails ? "alignas type layout: FAIL" : "alignas type layout: ok");
    return fails != 0;
}
