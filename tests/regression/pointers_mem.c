/* pointer arithmetic, casts, function pointers, arrays,
 * string literals, memcpy/memset behavior. */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

static int add(int a, int b) { return a + b; }
static int mul(int a, int b) { return a * b; }
static int apply(int (*f)(int,int), int a, int b) { return f(a, b); }

int main(void) {
    int arr[8];
    for (int i = 0; i < 8; i++) arr[i] = i * i;

    /* pointer arithmetic */
    int *p = arr;
    if (*p != 0) return 1;
    if (*(p + 3) != 9) return 2;
    if (p[5] != 25) return 3;
    if (&arr[7] - &arr[2] != 5) return 4;
    if ((char*)&arr[4] - (char*)&arr[0] != 16) return 5;
    p += 2; if (*p != 4) return 6;
    p -= 1; if (*p != 1) return 7;

    /* null checks */
    int *np = 0;
    if (np != 0) return 8;
    if (np) return 9;

    /* pointer casts */
    uintptr_t addr = (uintptr_t)&arr[3];
    if (*(int*)addr != 9) return 10;

    /* function pointers */
    if (apply(add, 3, 4) != 7) return 11;
    if (apply(mul, 3, 4) != 12) return 12;
    int (*fp)(int,int) = add;
    if (fp(10, 20) != 30) return 13;
    fp = mul;
    if (fp(10, 20) != 200) return 14;

    /* strings */
    const char *s = "hello";
    if (s[0] != 'h' || s[4] != 'o' || s[5] != '\0') return 15;
    char buf[32];
    strcpy(buf, "world");
    if (strcmp(buf, "world") != 0) return 16;
    if (strlen("abcdef") != 6) return 17;
    char *cat = "a" "b" "c";         /* adjacent literals */
    if (strcmp(cat, "abc") != 0) return 18;

    /* memcpy exactness (16/32-byte paths) */
    int src[16], dst[16];
    for (int i = 0; i < 16; i++) src[i] = i * 7;
    memcpy(dst, src, 64);             /* 64 bytes -> copies */
    if (memcmp(dst, src, 64) != 0) return 19;
    memcpy(dst, src, 16);             /* 16-byte path */
    if (memcmp(dst, src, 16) != 0) return 20;
    memcpy(dst, src, 32);             /* 32-byte path */
    if (memcmp(dst, src, 32) != 0) return 21;

    /* memset: 16 bytes = 4 ints */
    memset(dst, 0xAB, 16);
    for (int i = 0; i < 4; i++) if ((unsigned char)dst[i] != 0xAB) return 22;

    /* restrict/volatile pointers basic */
    volatile int v = 5;
    int *vp = (int*)&v;
    if (*vp != 5) return 23;
    *vp = 7;
    if (v != 7) return 24;

    printf("OK pointers_mem\n");
    return 0;
}
