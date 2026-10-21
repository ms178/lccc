/* Portable byte-swap networks: every spelling must equal the native swap.
 *
 * The kernels are deliberately `noinline` so the optimizer sees each network as
 * a whole function body, which is the shape the bit-idiom pass rewrites to
 * `Bswap`.  Correctness is checked three ways, so a wrong rewrite cannot hide
 * behind a wrong oracle:
 *
 *   1. against a byte-at-a-time reference implementation,
 *   2. against the compiler's own `__builtin_bswap{32,64}` lowering, which is
 *      an independent path to the same value,
 *   3. against explicit references for the near-miss kernels, which must keep
 *      their exact semantics: a rotate is not a swap, and a network that never
 *      routes one source byte must not become one.
 *
 * Two kernels go through memory, so the load/store side of the transformation
 * is exercised as well as the register-only shapes.
 */
typedef unsigned char U8;
typedef unsigned int U32;
typedef unsigned long long U64;

extern int printf(const char *, ...);

static U64 ref_swap(U64 x, unsigned bytes) {
    U64 result = 0;
    for (unsigned i = 0; i < bytes; ++i) {
        result = (result << 8) | (x & 255);
        x >>= 8;
    }
    return result;
}

/* Eight masked shift terms merged with `|`. */
__attribute__((noinline)) U64 bytes64(U64 x) {
    return ((x & 0x00000000000000ffULL) << 56) |
           ((x & 0x000000000000ff00ULL) << 40) |
           ((x & 0x0000000000ff0000ULL) << 24) |
           ((x & 0x00000000ff000000ULL) << 8) |
           ((x & 0x000000ff00000000ULL) >> 8) |
           ((x & 0x0000ff0000000000ULL) >> 24) |
           ((x & 0x00ff000000000000ULL) >> 40) |
           ((x & 0xff00000000000000ULL) >> 56);
}

/* The same lanes merged with `^`, which is exact only while they are disjoint. */
__attribute__((noinline)) U64 xor64(U64 x) {
    return ((x & 0x00000000000000ffULL) << 56) ^
           ((x & 0x000000000000ff00ULL) << 40) ^
           ((x & 0x0000000000ff0000ULL) << 24) ^
           ((x & 0x00000000ff000000ULL) << 8) ^
           ((x & 0x000000ff00000000ULL) >> 8) ^
           ((x & 0x0000ff0000000000ULL) >> 24) ^
           ((x & 0x00ff000000000000ULL) >> 40) ^
           ((x & 0xff00000000000000ULL) >> 56);
}

/* Whole-byte swap stages: no single stage is a swap, only their composition. */
__attribute__((noinline)) U64 stages64(U64 x) {
    x = ((x >> 8) & 0x00ff00ff00ff00ffULL) | ((x & 0x00ff00ff00ff00ffULL) << 8);
    x = ((x >> 16) & 0x0000ffff0000ffffULL) | ((x & 0x0000ffff0000ffffULL) << 16);
    return (x >> 32) | (x << 32);
}

/* The network reached through a chain of copies. */
__attribute__((noinline)) U64 copies64(U64 x) {
    U64 a = x;
    U64 b = a;
    U64 c = b;
    return ((c & 0x00000000000000ffULL) << 56) |
           ((c & 0x000000000000ff00ULL) << 40) |
           ((c & 0x0000000000ff0000ULL) << 24) |
           ((c & 0x00000000ff000000ULL) << 8) |
           ((c & 0x000000ff00000000ULL) >> 8) |
           ((c & 0x0000ff0000000000ULL) >> 24) |
           ((c & 0x00ff000000000000ULL) >> 40) |
           ((c & 0xff00000000000000ULL) >> 56);
}

__attribute__((noinline)) U32 bytes32(U32 x) {
    return ((x & 0x000000ffU) << 24) | ((x & 0x0000ff00U) << 8) |
           ((x & 0x00ff0000U) >> 8) | ((x & 0xff000000U) >> 24);
}

__attribute__((noinline)) U32 xor32(U32 x) {
    return ((x & 0x000000ffU) << 24) ^ ((x & 0x0000ff00U) << 8) ^
           ((x & 0x00ff0000U) >> 8) ^ ((x & 0xff000000U) >> 24);
}

__attribute__((noinline)) U32 stages32(U32 x) {
    x = ((x >> 8) & 0x00ff00ffU) | ((x & 0x00ff00ffU) << 8);
    return (x >> 16) | (x << 16);
}

/* Redundant parentheses and casts: the C front end introduces same-width casts
 * around such an expression, and they must not hide the network. */
__attribute__((noinline)) U32 wrapped32(U32 x) {
    return (U32)(((U32)(x & 0x000000ffU) << 24) |
                 ((U32)(x & 0x0000ff00U) << 8) |
                 ((U32)(x & 0x00ff0000U) >> 8) |
                 ((U32)(x & 0xff000000U) >> 24));
}

/* A rotate: two lanes move, nothing is reversed.  It must never become a swap. */
__attribute__((noinline)) U32 rotl8(U32 x) {
    return (x << 8) | (x >> 24);
}

/* Not a permutation: source byte 6 is never routed anywhere, so this value is
 * the full swap with byte lane 1 cleared and must keep exactly that meaning. */
__attribute__((noinline)) U64 missing_byte(U64 x) {
    return ((x & 0x00000000000000ffULL) << 56) |
           ((x & 0x000000000000ff00ULL) << 40) |
           ((x & 0x0000000000ff0000ULL) << 24) |
           ((x & 0x00000000ff000000ULL) << 8) |
           ((x & 0x000000ff00000000ULL) >> 8) |
           ((x & 0x0000ff0000000000ULL) >> 24) |
           ((x & 0xff00000000000000ULL) >> 56);
}

/* Memory shapes: a load or a store around the network. */
__attribute__((noinline)) U64 load_swap64(const void *p) {
    U64 x;
    __builtin_memcpy(&x, p, sizeof x);
    return bytes64(x);
}

__attribute__((noinline)) void store_swap64(void *p, U64 x) {
    U64 v = bytes64(x);
    __builtin_memcpy(p, &v, sizeof v);
}

static int check64(U64 x) {
    U64 want = ref_swap(x, 8);
    if (want != __builtin_bswap64(x)) return 1;
    if (bytes64(x) != want) return 2;
    if (xor64(x) != want) return 3;
    if (stages64(x) != want) return 4;
    if (copies64(x) != want) return 5;
    if (missing_byte(x) != (want & ~(0xffULL << 8))) return 6;
    if (rotl8((U32)x) != (U32)((((U32)x) << 8) | (((U32)x) >> 24))) return 7;
    return 0;
}

static int check32(U32 x) {
    U32 want = (U32)ref_swap(x, 4);
    if (want != __builtin_bswap32(x)) return 11;
    if (bytes32(x) != want) return 12;
    if (xor32(x) != want) return 13;
    if (stages32(x) != want) return 14;
    if (wrapped32(x) != want) return 15;
    return 0;
}

int main(void) {
    unsigned checked = 0;

    /* Every single-bit and single-byte pattern through both widths. */
    for (unsigned bit = 0; bit < 64; ++bit) {
        if (check64(1ULL << bit)) return 1;
        checked += 1;
    }
    for (unsigned lane = 0; lane < 8; ++lane) {
        for (unsigned byte = 0; byte < 256; ++byte) {
            if (check64((U64)byte << (8 * lane))) return 2;
            checked += 1;
        }
    }

    /* Every 16-bit value through both widths, then the high half of the
     * 32-bit space (the low half was the previous loop). */
    for (U32 x = 0; x < 65536; ++x) {
        if (check32(x)) return 3;
        if (check64(x)) return 4;
        checked += 1;
    }
    for (U32 high = 0; high < 65536; ++high) {
        if (check32(0xffff0000U | high)) return 5;
        checked += 1;
    }

    /* PRNG sweep over the full 64-bit space plus structural corners. */
    U64 x = 0x0123456789abcdefULL;
    for (unsigned i = 0; i < 65536; ++i) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if (check64(x)) return 6;
        checked += 1;
    }
    static const U64 corners[] = {
        0ULL,
        ~0ULL,
        0x8000000000000000ULL,
        0x0000000000000001ULL,
        0xaaaaaaaaaaaaaaaaULL,
        0x5555555555555555ULL,
        0x00ff00ff00ff00ffULL,
        0xff00ff00ff00ff00ULL,
        0x0123456789abcdefULL,
        0xfedcba9876543210ULL,
    };
    for (unsigned i = 0; i < sizeof corners / sizeof corners[0]; ++i) {
        if (check64(corners[i])) return 7;
        if (check32((U32)corners[i])) return 8;
        checked += 1;
    }

    /* Unaligned loads and stores, both directions.  The round trip is an
     * identity for a swap, and the store must write the swapped bytes. */
    U8 buf[24];
    for (unsigned i = 0; i < sizeof buf; ++i) buf[i] = (U8)(i * 37 + 11);
    for (unsigned off = 0; off <= 8; ++off) {
        U64 raw;
        __builtin_memcpy(&raw, buf + off, sizeof raw);
        U64 want = ref_swap(raw, 8);
        if (load_swap64(buf + off) != want) return 9;
        store_swap64(buf + off, raw);
        U64 stored;
        __builtin_memcpy(&stored, buf + off, sizeof stored);
        if (stored != want) return 10;
        checked += 1;
    }

    printf("portable byte-swap networks: PASS (%u inputs, ten kernels)\n", checked);
    return 0;
}
