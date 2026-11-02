#include <limits.h>
#include <stdint.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

NOINLINE int s8_low(signed char x) {
    switch (x) {
    case -128: return 1;
    case -127: return 0;
    case -126: return 1;
    case -125: return 0;
    default: return 0;
    }
}

NOINLINE int s8_vowel(signed char x) {
    switch (x) {
    case 'a': case 'e': case 'i': case 'o': case 'u':
    case 'A': case 'E': case 'I': case 'O': case 'U':
        return 1;
    default:
        return 0;
    }
}

NOINLINE int s8_high(signed char x) {
    switch (x) {
    case 120: return 1;
    case 121: return 0;
    case 122: return 1;
    case 127: return 1;
    default: return 0;
    }
}

NOINLINE int s8_out_of_domain(signed char x) {
    switch (x) {
    case 120: return 1;
    case 121: return 0;
    case 130: return 1;
    default: return 0;
    }
}

NOINLINE int u8_top(unsigned char x) {
    switch (x) {
    case 250: return 1;
    case 251: return 0;
    case 252: return 1;
    case 255: return 1;
    default: return 0;
    }
}

NOINLINE int s16_low(int16_t x) {
    switch (x) {
    case -32768: return -7;
    case -32767: return 11;
    case -32705: return -7;
    default: return 11;
    }
}

NOINLINE int s16_high(int16_t x) {
    switch (x) {
    case 32704: return -7;
    case 32705: return 11;
    case 32767: return -7;
    default: return 11;
    }
}

NOINLINE int u16_top(uint16_t x) {
    switch (x) {
    case 65505: return 1;
    case 65506: return 0;
    case 65535: return 1;
    default: return 0;
    }
}

NOINLINE int i32_small(uint32_t x) {
    switch (x) {
    case 10: case 12: case 15: case 20: return 42;
    case 11: case 14: return 7;
    default: return 7;
    }
}

NOINLINE int i32_low(int32_t x) {
    switch (x) {
    case INT_MIN: return 42;
    case INT_MIN + 2: return 7;
    case INT_MIN + 63: return 42;
    default: return 7;
    }
}

NOINLINE int i32_high(int32_t x) {
    switch (x) {
    case INT_MAX - 63: return 42;
    case INT_MAX - 2: return 7;
    case INT_MAX: return 42;
    default: return 7;
    }
}

NOINLINE int u32_top(uint32_t x) {
    switch (x) {
    case UINT_MAX - 63: return 1;
    case UINT_MAX - 2: return 0;
    case UINT_MAX: return 1;
    default: return 0;
    }
}

#if defined(__x86_64__)
typedef int (*s8_fn)(signed char);

/* Call a narrow function with deliberately non-canonical upper argument bits.
 * Only the low byte is a signed-char argument; the other 56 bits vary. */
__asm__(
    ".text\n"
    ".p2align 4\n"
    ".globl call_dirty_s8\n"
    ".type call_dirty_s8,@function\n"
    "call_dirty_s8:\n"
    "    subq $8, %rsp\n"
    "    movq %rdi, %rax\n"
    "    movq %rsi, %rdi\n"
    "    movzbl %dl, %edx\n"
    "    orq %rdx, %rdi\n"
    "    call *%rax\n"
    "    addq $8, %rsp\n"
    "    ret\n"
    ".size call_dirty_s8, .-call_dirty_s8\n");

extern int call_dirty_s8(s8_fn fn, unsigned long long payload, unsigned char low);

typedef int (*s16_fn)(int16_t);
typedef int (*u16_fn)(uint16_t);

/* Preserve the signed/zero-extended low 32 bits used by the narrow-argument
 * ABI, but vary the upper 32 bits. Keep distinct C function pointer types. */
__asm__(
    ".macro dirty16_stub name, extend\n"
    ".text\n"
    ".p2align 4\n"
    ".globl \\name\n"
    ".type \\name,@function\n"
    "\\name:\n"
    "    subq $8, %rsp\n"
    "    movq %rdi, %rax\n"
    "    movq %rsi, %rdi\n"
    "    \\extend %dx, %edx\n"
    "    orq %rdx, %rdi\n"
    "    call *%rax\n"
    "    addq $8, %rsp\n"
    "    ret\n"
    ".size \\name, .-\\name\n"
    ".endm\n"
    "dirty16_stub call_dirty_s16, movswl\n"
    "dirty16_stub call_dirty_u16, movzwl\n"
    ".purgem dirty16_stub\n");

extern int call_dirty_s16(s16_fn fn, unsigned long long payload, uint16_t low);
extern int call_dirty_u16(u16_fn fn, unsigned long long payload, uint16_t low);

static const unsigned long long dirty_payloads[] = {
    0x0000000000000000ULL, 0x123456789abc0000ULL,
    0xffffffffffff0000ULL, 0x00000000ffff0000ULL,
    0x7fffffff80000000ULL, 0x8000000080000000ULL
};
#define DIRTY_PAYLOAD_COUNT (sizeof(dirty_payloads) / sizeof(dirty_payloads[0]))

static int check_dirty_s8_calls(void) {
    for (unsigned p = 0; p < DIRTY_PAYLOAD_COUNT; ++p) {
        for (unsigned low = 0; low <= UCHAR_MAX; ++low) {
            signed char x = (signed char)low;
            int vowel = x == 'a' || x == 'e' || x == 'i' || x == 'o' || x == 'u' ||
                        x == 'A' || x == 'E' || x == 'I' || x == 'O' || x == 'U';
            if (call_dirty_s8(s8_vowel, dirty_payloads[p], (unsigned char)low) != vowel ||
                call_dirty_s8(s8_low, dirty_payloads[p], (unsigned char)low) !=
                    (x == -128 || x == -126) ||
                call_dirty_s8(s8_high, dirty_payloads[p], (unsigned char)low) !=
                    (x == 120 || x == 122 || x == 127) ||
                call_dirty_s8(s8_out_of_domain, dirty_payloads[p], (unsigned char)low) !=
                    (x == 120))
                return 1;
        }
    }
    return 0;
}

static int check_dirty_16_calls(void) {
    for (unsigned p = 0; p < DIRTY_PAYLOAD_COUNT; ++p) {
        unsigned long long payload = dirty_payloads[p] & 0xffffffff00000000ULL;
        for (unsigned low = 0; low <= USHRT_MAX; ++low) {
            int16_t s = (int16_t)low;
            uint16_t u = (uint16_t)low;
            int low_expected = (s == -32768 || s == -32705) ? -7 : 11;
            int high_expected = (s == 32704 || s == 32767) ? -7 : 11;
            if (call_dirty_s16(s16_low, payload, u) != low_expected ||
                call_dirty_s16(s16_high, payload, u) != high_expected ||
                call_dirty_u16(u16_top, payload, u) !=
                    (u == 65505 || u == 65535))
                return 1;
        }
    }
    return 0;
}
#endif

static unsigned long long h = 1469598103934665603ULL;

static void mix(unsigned long long x) {
    h ^= x + 0x9e3779b97f4a7c15ULL + (h << 6) + (h >> 2);
    h *= 1099511628211ULL;
}

int main(void) {
    for (int x = -128; x <= 127; ++x) {
        mix((unsigned)s8_low((signed char)x));
        mix((unsigned)s8_vowel((signed char)x));
        mix((unsigned)s8_high((signed char)x));
        mix((unsigned)s8_out_of_domain((signed char)x));
    }
    for (unsigned x = 0; x <= UCHAR_MAX; ++x)
        mix(u8_top((unsigned char)x));
    for (int x = -32768; x <= 32767; ++x) {
        mix((unsigned)s16_low((int16_t)x));
        mix((unsigned)s16_high((int16_t)x));
        mix(u16_top((uint16_t)x));
    }
    for (uint32_t x = 0; x < 100; ++x)
        mix((unsigned)i32_small(x));
    const int32_t signed_edges[] = {
        INT_MIN, INT_MIN + 1, INT_MIN + 2, INT_MIN + 62, INT_MIN + 63,
        INT_MIN + 64, -1, 0, 1, INT_MAX - 64, INT_MAX - 63,
        INT_MAX - 2, INT_MAX - 1, INT_MAX
    };
    for (unsigned i = 0; i < sizeof(signed_edges) / sizeof(signed_edges[0]); ++i) {
        mix((unsigned)i32_low(signed_edges[i]));
        mix((unsigned)i32_high(signed_edges[i]));
    }
    const uint32_t unsigned_edges[] = {
        0, 1, 63, 64, UINT_MAX - 64, UINT_MAX - 63,
        UINT_MAX - 2, UINT_MAX - 1, UINT_MAX
    };
    for (unsigned i = 0; i < sizeof(unsigned_edges) / sizeof(unsigned_edges[0]); ++i)
        mix(u32_top(unsigned_edges[i]));
#if defined(__x86_64__)
    if (check_dirty_s8_calls() || check_dirty_16_calls())
        return 2;
#endif
    printf("%016llx\n", h);
    return h != 0x1279b3e25ccfc38fULL;
}
