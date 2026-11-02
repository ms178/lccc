/* The AVX2-specific strided reduction must preserve its scalar semantics on
   i686, AArch64 and RISC-V as well as x86-64. No libc needed for QEMU. */
static unsigned words[259];

__attribute__((noinline))
static unsigned reduce(const unsigned char *p) {
    unsigned sum = 0;
    for (unsigned long long i = 0; i < 1024; i += 8)
        sum += *(const unsigned *)(p + i) ^ (unsigned)i;
    return sum;
}

int main(void) {
    unsigned expected = 0;
    for (unsigned i = 0; i < 259; ++i) words[i] = i * 2654435761u + 17u;
    for (unsigned i = 0; i < 128; ++i) expected += words[i * 2] ^ (i * 8);
    if (reduce((const unsigned char *)words) != expected) return 1;
    expected = 0;
    for (unsigned i = 0; i < 128; ++i) expected += words[i * 2 + 2] ^ (i * 8);
    if (reduce((const unsigned char *)(words + 2)) != expected) return 2;
    return 0;
}
