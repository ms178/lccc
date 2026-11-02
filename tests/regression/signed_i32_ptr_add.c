/*
 * P0: simplify_cast must not Copy Cast(I32->I64->I32).
 *
 * Signed 32-bit homes are often zero-extended in the 64-bit register
 * (`movl` / 32-bit ALU). The round-trip sign-extends (`movslq`). Copy
 * keeps the zext high bits, so a later 64-bit use (pointer += length)
 * sees 0x00000000FFFFFFFF instead of -1. That miscompiled the kernel
 * preboot ZSTD decoder at -O1+ (`ZSTD-compressed data is corrupt`).
 *
 * Unsigned round-trips stay Copy (already zext-canonical).
 */
#include <stdio.h>
#include <string.h>

__attribute__((noinline))
int roundtrip_i32(int x)
{
    return (int)(long)x;
}

__attribute__((noinline))
char *add_roundtrip(char *p, int n)
{
    return p + roundtrip_i32(n);
}

__attribute__((noinline))
unsigned roundtrip_u32(unsigned x)
{
    return (unsigned)(long)x;
}

int main(void)
{
    char buf[16];
    char *mid = buf + 8;
    char *got;
    int fails = 0;
    unsigned u;

    memset(buf, 0xAB, sizeof(buf));

    got = add_roundtrip(mid, -1);
    if (got != mid - 1) {
        printf("FAIL signed ptr add: got %ld expected -1\n",
               (long)(got - mid));
        fails++;
    }

    got = add_roundtrip(mid, -8);
    if (got != buf) {
        printf("FAIL signed ptr add -8: got %ld expected -8\n",
               (long)(got - mid));
        fails++;
    }

    got = add_roundtrip(mid, 3);
    if (got != mid + 3) {
        printf("FAIL signed ptr add +3: got %ld expected 3\n",
               (long)(got - mid));
        fails++;
    }

    if (roundtrip_i32(-1) != -1) {
        printf("FAIL roundtrip_i32(-1)=%d\n", roundtrip_i32(-1));
        fails++;
    }

    u = roundtrip_u32(0xFFFFFFFFu);
    if (u != 0xFFFFFFFFu) {
        printf("FAIL unsigned roundtrip=0x%x\n", u);
        fails++;
    }

    if (fails) {
        printf("%d FAIL\n", fails);
        return 1;
    }
    printf("OK\n");
    return 0;
}
