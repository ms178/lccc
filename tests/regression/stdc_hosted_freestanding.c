/* -ffreestanding: __STDC_HOSTED__ is 0 (C17 6.10.8.1), plain library names
 * are not assumed, and the __builtin_ spellings still are (GCC 16.2).
 * Compared against GCC 16.2 (same flags; the program itself links libc). */
int printf(const char *, ...);

#if __STDC_HOSTED__ != 0
#error "-ffreestanding must define __STDC_HOSTED__ as 0"
#endif

int main(void) {
    unsigned char b[24];
    __builtin_memset(b, 0x5a, sizeof b);
    __builtin_memcpy(b + 4, "lccc", 4);
    printf("%d %c%c %x %x\n", __STDC_HOSTED__, b[4], b[7], b[0], b[23]);
    return 0;
}
