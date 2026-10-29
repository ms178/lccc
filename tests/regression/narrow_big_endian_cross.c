/* No libc: run the promoted-field semantics under cross-target QEMU as well. */
__attribute__((noinline))
static unsigned read_u16(const unsigned char *p) {
    return ((unsigned)p[0] << 8) | (unsigned)p[1];
}

__attribute__((noinline))
static long long read_s16(const unsigned char *p) {
    return (short)(((unsigned short)p[0] << 8) | (unsigned short)p[1]);
}

__attribute__((noinline))
static unsigned long long read_u32_wide(const unsigned char *p) {
    return ((unsigned long long)p[0] << 24) | ((unsigned long long)p[1] << 16)
         | ((unsigned long long)p[2] << 8) | (unsigned long long)p[3];
}

int main(void) {
    unsigned char bytes[12] = {0};
    for (unsigned n = 0; n <= 65535u; ++n) {
        unsigned off = n & 7u;
        unsigned value = n * 2654435761u;
        bytes[off] = (unsigned char)(n >> 8);
        bytes[off + 1] = (unsigned char)n;
        if (read_u16(bytes + off) != n) return 1;
        if (read_s16(bytes + off) != (long long)((int)(n ^ 32768u) - 32768))
            return 2;
        bytes[off] = (unsigned char)(value >> 24);
        bytes[off + 1] = (unsigned char)(value >> 16);
        bytes[off + 2] = (unsigned char)(value >> 8);
        bytes[off + 3] = (unsigned char)value;
        if (read_u32_wide(bytes + off) != (unsigned long long)value) return 3;
    }
    return 0;
}
