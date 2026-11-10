/* Freestanding binary16 -> binary32 probe, for the i686 leg of
 * `check_f16_conversion_parity.sh`.
 *
 * Same property as the hosted fixture -- `-mf16c` must not change a value --
 * but with no C library, so it can be EXECUTED on a backend whose libc this
 * host does not have.  The defect it exists for was measured on exactly that
 * backend: `-m32 -mf16c -mavx` defined `__F16C__`, then took the software
 * conversion path because the effective feature set never received `F16c`
 * (`x86_isa()` pins the AVX ceiling to NONE for i686).  The option bought
 * nothing while claiming the extension, and only a comparison of the two
 * builds could see it: both compute correct values, so a value-only test
 * without the assembly check would pass.
 *
 * Prints one `pattern bits` row per binary16 class.  No formatting library:
 * the two hex fields are written here.
 */
typedef unsigned int u32;

static int sys_write(const char *p, u32 n) {
    int r;
    __asm__ volatile("int $0x80" : "=a"(r) : "a"(4), "b"(1), "c"(p), "d"(n) : "memory");
    return r;
}

static char buf[1024];
static char *o = buf;

static char *put_hex16(char *p, unsigned v) {
    static const char d[] = "0123456789abcdef";
    for (int shift = 12; shift >= 0; shift -= 4) {
        *p++ = d[(v >> shift) & 0xF];
    }
    return p;
}

static char *put_hex32(char *p, unsigned v) {
    static const char d[] = "0123456789abcdef";
    for (int shift = 28; shift >= 0; shift -= 4) {
        *p++ = d[(v >> shift) & 0xF];
    }
    return p;
}

/* Every binary16 class worth checking: both zeros, a subnormal, the
 * subnormal/normal boundary, normals of both signs, the largest finite, both
 * infinities, a quiet NaN, and three ordinary values. */
static const unsigned short patterns[] = {
    0x0000, 0x8000, 0x0001, 0x03FF, 0x0400, 0x3C00,
    0xBC00, 0x7BFF, 0x7C00, 0xFC00, 0x3555, 0x4248,
    0x5140, 0x5B9E,
};

void _start(void) {
    const unsigned n = sizeof patterns / sizeof *patterns;
    for (unsigned i = 0; i < n; i++) {
        _Float16 h;
        __builtin_memcpy(&h, &patterns[i], 2);
        float f = (float)h;
        unsigned bits;
        __builtin_memcpy(&bits, &f, 4);
        o = put_hex16(o, patterns[i]);
        *o++ = ' ';
        o = put_hex32(o, bits);
        *o++ = '\n';
    }
    sys_write(buf, (u32)(o - buf));
    for (;;) {
        __asm__ volatile("int $0x80" : : "a"(252), "b"(0) : "memory");
    }
}
