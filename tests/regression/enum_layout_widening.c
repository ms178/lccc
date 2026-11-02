/* Enum layout semantics: the compatible integer type follows the completed
 * definition's value range (GCC widening rule), packed enums honor their
 * minimum width, and struct layouts inherit enum alignment.  Pairs with
 * enum_static_assert_ice.c (compile-time contracts) — this file checks the
 * runtime-visible sizes. */
#include <stdio.h>

enum Big { B0 = 0x100000000LL };
enum Neg { N1 = -1, N2 = 4000000000u };
enum AllNeg { A1 = -5, A2 = -1 };
enum __attribute__((packed)) PK { PKA = 1, PKB = 300 };
struct WithEnum { char c; enum Big b; };
struct __attribute__((packed)) PackedWithEnum { char c; enum PK p; };

int main(void) {
    printf("alignof(Big)=%zu\n", _Alignof(enum Big));
    printf("sizeof(Neg)=%zu\n", sizeof(enum Neg));
    printf("sizeof(AllNeg)=%zu\n", sizeof(enum AllNeg));
    printf("sizeof(PK)=%zu\n", sizeof(enum PK));
    printf("sizeof(WithEnum)=%zu\n", sizeof(struct WithEnum));
    printf("sizeof(PackedWithEnum)=%zu\n", sizeof(struct PackedWithEnum));
    enum Neg n = N2;
    printf("N2=%u\n", (unsigned)n);
    printf("signedness=%d\n", (int)((enum AllNeg)-1 < 0));
    return 0;
}
