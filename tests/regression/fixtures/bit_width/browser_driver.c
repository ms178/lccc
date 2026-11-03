/* driver.c -- deterministic checksum over every kernel of browser_bits.c.
 *
 * Freestanding (no libc): `browser_bits_checksum()` folds every kernel result
 * and every byte the kernels modify into one 64-bit FNV-style accumulator.
 * Only inputs C defines are used (shift counts below the width, no `1 << 31`
 * into an int, `sby - 1` never overflows), and every value folded in is the
 * same on LP64 and ILP32 little-endian targets: the `unsigned long` digit
 * arrays are observed byte by byte, `long occurs` through its low 32 bits, and
 * `srtp_rdb_check` windows stay below 2^31 so the ILP32 `size_t` sum cannot
 * wrap.  One expected value therefore covers x86-64, i686, AArch64 and RV64.
 */
#ifndef BROWSER_BITS_TYPES /* browser_combined.c includes the kernels first */
typedef unsigned char u8;
typedef unsigned int u32;
typedef unsigned long long u64;
typedef __SIZE_TYPE__ size_t;
typedef union {
    u8 v8[16];
    unsigned short v16[8];
    u32 v32[4];
    u64 v64[2];
} v128_t;
typedef struct {
    u32 window_start;
    v128_t bitmask;
} srtp_rdb_t;
struct vstate {
    void *cont;
    void *node;
    long occurs;
    unsigned char depth;
    unsigned char state;
};
#endif

int t_ne(u32, u32);
int t_eq(u32, u32);
int t_eq64(u64, u32);
int t_shr(u32, u32);
int t_shr_not(u32, u32);
int t_not_ne(int, int);
int t_const_mask(int);
int t_msb(const u8 *, u32);
void b_ne(u32, u32);
void b_eq(u32, u32);
void b_shr(u32, u32);
void b_not(int, int);
void b_const_mask(int);
void b_idx_mask(int, int);
int ff_test_clear(u32 *, u32);
int ff_test_clear64(u64 *, u32);
int ff_test_set(u32 *, u32);
int ff_test_toggle(u32 *, u32);
void ff_branch_clear(u32 *, u32);
int ff_visit(u32 *, u32);
u32 shared_four(u32, u32, u32, u32, u32);
int sqlite_maskbit(u64, int);
int sqlite_dbmask(u32, int);
void sqlite_uses_btree(u32 *, u32 *, int, int);
int sqlite_bitvec_test(const u8 *, u32);
void sqlite_bitvec_set(u8 *, u32);
void sqlite_bitvec_clear(u8 *, u32);
int sctp_present(const u8 *, u32);
void sctp_set(u8 *, u32);
void sctp_unset(u8 *, u32);
u32 srtp_get(const v128_t *, u32);
void srtp_set(v128_t *, u32);
void srtp_clear(v128_t *, u32);
int srtp_rdb_check(const srtp_rdb_t *, u32);
int icu_lead3(int, u8);
int icu_lead4(int, u8);
void brotli_mark(u32 *, size_t);
u32 brotli_trivial(const u32 *, size_t);
int ft_is32_clear(const u8 *, u32);
int ft_is32_range_set(const u8 *, u32, u32);
int ft_mask_any3(const u8 *, u32, u32, u32);
void ft_mask_set(u8 *, u32);
void nss_set_bit(unsigned long *, u32, u32);
int nss_get_bit(const unsigned long *, u32);
int libxml_occurs(struct vstate *);
u32 lzma_count(const u8 *, u32);
u32 lzma_defined_sum(const u8 *, u32, const u32 *);

static u64 acc = 14695981039346656037ull;
static u32 seed = 0x2545f491u;

static void mix(u64 v) { acc = (acc ^ v) * 1099511628211ull; }
void hit(u32 v) { mix(0x9e3779b9u ^ v); }
static u32 rnd(void) {
    seed ^= seed << 13;
    seed ^= seed >> 17;
    seed ^= seed << 5;
    return seed;
}
/* Two draws in a fixed order: C leaves the operand and argument evaluation
 * order unspecified, so no expression below ever calls rnd() twice. */
static u64 rnd64(void) {
    u64 hi = rnd();
    return (hi << 32) | rnd();
}
static void mix_bytes(const void *p, u32 n) {
    const u8 *b = (const u8 *)p;
    u32 i;
    for (i = 0; i < n; i++)
        mix(b[i]);
}

static const u32 patterns[] = {0u, 0xffffffffu, 0x80000001u, 0x55555555u, 0xaaaaaaaau,
                               0x0000ffffu, 0xffff0000u, 0x12345678u, 0xfedcba98u, 1u};
#define NPAT (sizeof(patterns) / sizeof(patterns[0]))

static void single_tests(void) {
    u32 p, n, r;
    for (p = 0; p < NPAT + 6; p++) {
        u32 x = p < NPAT ? patterns[p] : rnd();
        u64 x64 = ((u64)rnd() << 32) | x; /* one draw */
        for (n = 0; n < 32; n++) {
            mix((u64)t_ne(x, n) | (u64)t_eq(x, n) << 1 | (u64)t_shr(x, n) << 2 |
                (u64)t_shr_not(x, n) << 3 | (u64)t_eq64(x64, n) << 4 |
                (u64)t_eq64(x64, n + 32) << 5);
            b_ne(x, n);
            b_eq(x, n);
            b_shr(x, n);
        }
        for (r = 1; r < 70; r++) {
            mix((u64)(u32)t_not_ne((int)x, (int)r));
            b_not((int)x, (int)r);
            b_idx_mask((int)x, (int)r);
        }
    }
    for (n = 0; n < 31; n++) {
        mix((u64)(u32)t_const_mask((int)n) << 7);
        b_const_mask((int)n);
    }
}

static void test_and_modify(void) {
    u32 w[4], n, p, i;
    u64 w64;
    u32 visited[8];
    for (p = 0; p < NPAT + 4; p++) {
        u32 x = p < NPAT ? patterns[p] : rnd();
        for (n = 0; n < 32; n++) {
            w[0] = w[1] = w[2] = w[3] = x;
            mix((u64)(u32)ff_test_clear(&w[0], n) | (u64)(u32)ff_test_set(&w[1], n) << 1 |
                (u64)(u32)ff_test_toggle(&w[2], n) << 2);
            ff_branch_clear(&w[3], n);
            mix_bytes(w, sizeof(w));
            mix(shared_four(x, ~x, x ^ 0x5a5a5a5au, rnd(), n));
        }
        for (n = 0; n < 64; n++) {
            w64 = ((u64)x << 32) | (x ^ 0x0f0f0f0fu);
            mix((u64)(u32)ff_test_clear64(&w64, n));
            mix(w64);
        }
    }
    for (i = 0; i < 8; i++)
        visited[i] = rnd() & rnd();
    for (i = 0; i < 600; i++)
        mix((u64)(u32)ff_visit(visited, rnd() & 255));
    mix_bytes(visited, sizeof(visited));
}

static void sqlite_tests(void) {
    u8 bitmap[64];
    u32 i, btree = 0, lock = 0;
    int c;
    for (i = 0; i < 12; i++) {
        u64 mask = rnd64();
        for (c = 0; c < 72; c++)
            mix((u64)(u32)sqlite_maskbit(mask, c));
    }
    for (i = 0; i < 12; i++) {
        u32 m = rnd();
        for (c = 0; c < 32; c++)
            mix((u64)(u32)sqlite_dbmask(m, c));
    }
    for (c = 0; c < 32; c++) {
        sqlite_uses_btree(&btree, &lock, c, (int)(rnd() & 1));
        mix(btree);
        mix(lock);
    }
    for (i = 0; i < 64; i++)
        bitmap[i] = (u8)rnd();
    for (i = 0; i < 900; i++) {
        u32 b = rnd() & 511;
        switch (rnd() % 3) {
        case 0: mix((u64)(u32)sqlite_bitvec_test(bitmap, b)); break;
        case 1: sqlite_bitvec_set(bitmap, b); break;
        default: sqlite_bitvec_clear(bitmap, b); break;
        }
    }
    mix_bytes(bitmap, sizeof(bitmap));
}

static void sctp_tests(void) {
    u8 arry[64];
    u32 i;
    for (i = 0; i < 64; i++)
        arry[i] = (u8)rnd();
    for (i = 0; i < 900; i++) {
        u32 gap = rnd() & 511;
        switch (rnd() % 3) {
        case 0: mix((u64)(u32)sctp_present(arry, gap)); break;
        case 1: sctp_set(arry, gap); break;
        default: sctp_unset(arry, gap); break;
        }
    }
    mix_bytes(arry, sizeof(arry));
}

static void srtp_tests(void) {
    v128_t v;
    srtp_rdb_t rdb;
    u32 i;
    for (i = 0; i < 4; i++)
        v.v32[i] = rnd();
    for (i = 0; i < 700; i++) {
        u32 bit = rnd() & 127;
        switch (rnd() % 3) {
        case 0: mix(srtp_get(&v, bit)); break;
        case 1: srtp_set(&v, bit); break;
        default: srtp_clear(&v, bit); break;
        }
    }
    mix_bytes(&v, sizeof(v));
    for (i = 0; i < 300; i++) {
        u32 k;
        rdb.window_start = rnd() & 0x7fffffffu;
        for (k = 0; k < 4; k++)
            rdb.bitmask.v32[k] = rnd();
        for (k = 0; k < 8; k++) {
            u32 delta = rnd() % 300;
            u32 idx = (rnd() & 1) ? rdb.window_start + delta : rdb.window_start - delta;
            mix((u64)(u32)srtp_rdb_check(&rdb, idx));
        }
    }
}

static void icu_brotli_tests(void) {
    u32 lead, t1, ctx[8], i;
    for (lead = 0; lead < 256; lead++)
        for (t1 = 0; t1 < 256; t1 += 3)
            mix((u64)(u32)icu_lead3((int)lead, (u8)t1) | (u64)(u32)icu_lead4((int)lead, (u8)t1) << 1);
    for (i = 0; i < 8; i++)
        ctx[i] = 0;
    for (i = 0; i < 256; i++)
        if (rnd() & 1)
            brotli_mark(ctx, i);
    for (i = 0; i < 256; i++)
        mix(brotli_trivial(ctx, i));
    mix_bytes(ctx, sizeof(ctx));
}

static u8 is32[8192];

static void freetype_tests(void) {
    u8 bytes[16];
    u32 i;
    for (i = 0; i < 8192; i++)
        is32[i] = (u8)(rnd() | rnd());
    for (i = 0; i < 2000; i++)
        mix((u64)(u32)ft_is32_clear(is32, rnd()));
    for (i = 0; i < 300; i++) {
        u32 start = rnd();
        u32 count = rnd() & 15;
        mix((u64)(u32)ft_is32_range_set(is32, start, count));
    }
    for (i = 0; i < 16; i++)
        bytes[i] = (u8)(rnd() & rnd() & rnd());
    for (i = 0; i < 600; i++) {
        u32 h1 = rnd() & 127;
        u32 h2 = rnd() & 127;
        u32 h3 = rnd() & 127;
        mix((u64)(u32)ft_mask_any3(bytes, h1, h2, h3));
    }
    for (i = 0; i < 40; i++)
        ft_mask_set(bytes, rnd() & 127);
    mix_bytes(bytes, sizeof(bytes));
}

static void nss_libxml_lzma_tests(void) {
    union { /* 64 bytes on LP64 and ILP32 alike: 8 or 16 digits */
        unsigned long d[64 / sizeof(unsigned long)];
        u8 b[64];
    } u;
    struct vstate vs;
    u8 bits[32];
    u32 data[256], i, nbits = 8 * (u32)sizeof(u.d);
    for (i = 0; i < sizeof(u.b); i++)
        u.b[i] = (u8)rnd();
    for (i = 0; i < 800; i++) {
        u32 b = rnd() % nbits;
        if (rnd() & 1)
            nss_set_bit(u.d, b, rnd() & 1);
        else
            mix((u64)(u32)nss_get_bit(u.d, b));
    }
    mix_bytes(u.b, sizeof(u.b));
    for (i = 0; i < 600; i++) {
        vs.occurs = (long)(int)rnd();
        vs.depth = (unsigned char)(rnd() % 31);
        mix((u64)(u32)libxml_occurs(&vs));
        mix((u32)vs.occurs);
    }
    for (i = 0; i < 32; i++)
        bits[i] = (u8)rnd();
    for (i = 0; i < 256; i++)
        data[i] = rnd();
    for (i = 0; i <= 256; i += 7) {
        mix(lzma_count(bits, i));
        mix(lzma_defined_sum(bits, i, data));
    }
}

u64 browser_bits_checksum(void) {
    single_tests();
    test_and_modify();
    sqlite_tests();
    sctp_tests();
    srtp_tests();
    icu_brotli_tests();
    freetype_tests();
    nss_libxml_lzma_tests();
    for (u32 i = 0; i < 64; i++) {
        u64 x = rnd64();
        mix((u64)(u32)t_msb((const u8 *)&x, i));
    }
    return acc;
}
