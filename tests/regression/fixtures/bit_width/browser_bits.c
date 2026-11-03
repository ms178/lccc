/* browser_bits.c -- bit-index idioms transcribed from the pinned browser trees.
 *
 * Every kernel is a macro-expanded transcription of one source idiom; the
 * comment above it names the file and line in
 *
 *   chromium/chromium        66fd4204b000bb138183d18ae80cb479e48ffc63
 *   mozilla-firefox/firefox  141096d7cd9e6f6118d94d79ea0c1777839efe9e
 *
 * (`fetch_sources.sh` re-downloads exactly those files through the GitHub API
 * and `grep`s the lines back).  The generic `t_*` / `b_*` / `ff_*` rows are the
 * same idioms reduced to one operation each, so a count change can be traced
 * to one fold.  Freestanding on purpose: the cross drivers have no sysroot, so
 * the file carries its own typedefs and one external sink, `hit()`.
 *
 * Inputs for which C leaves a shift undefined (count >= width, or a set sign
 * bit in `1 << 31`) are never passed by `driver.c`.
 */
typedef unsigned char u8;
typedef unsigned int u32;
typedef unsigned long long u64;
typedef __SIZE_TYPE__ size_t;

void hit(u32 v);

/* dav1d src/levels.h:158 enum BlockSize, src/tables.h:70 and :86. */
#define BS_32x32 7
#define BS_32x16 8
#define BS_32x8 9
#define BS_16x32 11
#define BS_16x16 12
#define BS_16x8 13
#define BS_16x4 14
#define BS_8x32 15
#define BS_8x16 16
#define BS_8x8 17
#define BS_8x4 18
#define BS_4x16 19
#define BS_4x8 20
#define BS_4x4 21
#define CFL_ALLOWED_MASK                                                     \
    ((1 << BS_32x32) | (1 << BS_32x16) | (1 << BS_32x8) | (1 << BS_16x32) |  \
     (1 << BS_16x16) | (1 << BS_16x8) | (1 << BS_16x4) | (1 << BS_8x32) |    \
     (1 << BS_8x16) | (1 << BS_8x8) | (1 << BS_8x4) | (1 << BS_4x16) |       \
     (1 << BS_4x8) | (1 << BS_4x4))
#define WEDGE_ALLOWED_MASK                                                   \
    ((1 << BS_32x32) | (1 << BS_32x16) | (1 << BS_32x8) | (1 << BS_16x32) |  \
     (1 << BS_16x16) | (1 << BS_16x8) | (1 << BS_8x32) | (1 << BS_8x16) |    \
     (1 << BS_8x8))

/* ---- one test, predicate returned ---------------------------------------- */

/* dav1d src/decode.c:1117 `!!(cfl_allowed_mask & (1 << bs))`, variable mask. */
int t_ne(u32 x, u32 n) { return (x & (1u << n)) != 0; }
/* freetype src/sfnt/ttcmap.c:1868 `(is32[...] & bit) == 0`. */
int t_eq(u32 x, u32 n) { return (x & (1u << n)) == 0; }
/* sqlite3.c:93317 `(mask & MASKBIT(iCol))==0`, Bitmask is u64. */
int t_eq64(u64 x, u32 n) { return (x & (1ull << n)) == 0; }
/* nss freebl/mpi/mplogic.c:376 `(MP_DIGIT(a, ix) >> bit) & 1`. */
int t_shr(u32 x, u32 n) { return (x >> n) & 1; }
/* libsrtp crypto/replay/rdb.c:87 `v128_get_bit(...) == 1`, negated. */
int t_shr_not(u32 x, u32 n) { return ((x >> n) & 1) != 1; }
/* dav1d src/thread_task.c:820 `~prog & (1U << ((sby - 1) & 31))`. */
int t_not_ne(int prog, int sby) { return (~prog & (1u << ((sby - 1) & 31))) != 0; }
/* dav1d src/decode.c:1117 with the constant cfl_allowed_mask. */
int t_const_mask(int bs) { return !!(CFL_ALLOWED_MASK & (1 << bs)); }
/* lzma_sdk C/7z.h:75 SzBitArray_Check: MSB-first bit string. */
int t_msb(const u8 *p, u32 i) { return (p[i >> 3] & (0x80 >> (i & 7))) != 0; }

/* ---- one test, branch on it ---------------------------------------------- */

void b_ne(u32 x, u32 n) { if (x & (1u << n)) hit(n); }
void b_eq(u32 x, u32 n) { if (!(x & (1u << n))) hit(n); }
void b_shr(u32 x, u32 n) { if ((x >> n) & 1) hit(n); }
/* dav1d src/thread_task.c:820, the branch it guards. */
void b_not(int prog, int sby) { if (~prog & (1u << ((sby - 1) & 31))) hit((u32)sby); }
/* dav1d src/decode.c:1639 `if (wedge_allowed_mask & (1 << bs))`. */
void b_const_mask(int bs) { if (WEDGE_ALLOWED_MASK & (1 << bs)) hit((u32)bs); }
/* dav1d src/thread_task.c:649 `if (p1 & (1U << ((t->sby - 1) & 31)))`. */
void b_idx_mask(int p1, int sby) { if (p1 & (1u << ((sby - 1) & 31))) hit((u32)sby); }

/* ---- test and modify the same bit ------------------------------------------ */

int ff_test_clear(u32 *w, u32 n) {
    u32 m = 1u << n;
    int was = (*w & m) != 0;
    *w &= ~m;
    return was;
}
int ff_test_clear64(u64 *w, u32 n) {
    u64 m = 1ull << n;
    int was = (*w & m) != 0;
    *w &= ~m;
    return was;
}
int ff_test_set(u32 *w, u32 n) {
    int was = (*w >> n) & 1;
    *w |= 1u << n;
    return was;
}
int ff_test_toggle(u32 *w, u32 n) {
    int was = (*w >> n) & 1;
    *w ^= 1u << n;
    return was;
}
void ff_branch_clear(u32 *w, u32 n) {
    if (*w & (1u << n)) {
        *w &= ~(1u << n);
        hit(n);
    }
}
/* brotli dec/decode.c:1240 (set) and :1251-1252 (test) on one word array. */
int ff_visit(u32 *visited, u32 i) {
    if (visited[i >> 5] & (1u << (i & 31)))
        return 0;
    visited[i >> 5] |= 1u << (i & 31);
    return 1;
}
/* Four readers of one mask: set, clear, toggle and test. */
u32 shared_four(u32 a, u32 b, u32 c, u32 d, u32 n) {
    u32 m = 1u << n;
    return (a | m) + (b & ~m) + (c ^ m) + ((d & m) != 0);
}

/* ---- sqlite (firefox third_party/sqlite3/src/sqlite3.c) -------------------- */

/* :16451 MASKBIT, :93317 `if( iCol>=BMS || (mask & MASKBIT(iCol))==0 ) return 0;` */
int sqlite_maskbit(u64 mask, int iCol) {
    if (iCol >= 64 || (mask & (((u64)1) << (iCol))) == 0)
        return 0;
    return 1;
}
/* :20834 DbMaskTest (yDbMask is unsigned int), :89974 `i!=1 && DbMaskTest(...)`. */
int sqlite_dbmask(u32 lockMask, int i) {
    return i != 1 && (((lockMask) & (((u32)1) << (i))) != 0);
}
/* :20836 DbMaskSet, :89936-89939 sqlite3VdbeUsesBtree. */
void sqlite_uses_btree(u32 *btreeMask, u32 *lockMask, int i, int sharable) {
    ((*btreeMask) |= (((u32)1) << (i)));
    if (i != 1 && sharable)
        ((*lockMask) |= (((u32)1) << (i)));
}
/* :56414 Bitvec test (BITVEC_TELEM u8, BITVEC_SZELEM 8). */
int sqlite_bitvec_test(const u8 *aBitmap, u32 i) {
    return (aBitmap[i / 8] & (1 << (i & (8 - 1)))) != 0;
}
/* :56456 Bitvec set. */
void sqlite_bitvec_set(u8 *aBitmap, u32 i) { aBitmap[i / 8] |= 1 << (i & (8 - 1)); }
/* :56526 Bitvec clear. */
void sqlite_bitvec_clear(u8 *aBitmap, u32 i) {
    aBitmap[i / 8] &= ~(u8)(1 << (i & (8 - 1)));
}

/* ---- usrsctp (firefox netwerk/sctp/src/netinet/sctp_constants.h) ----------- */

/* :896 SCTP_IS_TSN_PRESENT */
int sctp_present(const u8 *arry, u32 gap) { return ((arry[(gap >> 3)] >> (gap & 0x07)) & 0x01); }
/* :897 SCTP_SET_TSN_PRESENT */
void sctp_set(u8 *arry, u32 gap) { (arry[(gap >> 3)] |= (0x01 << ((gap & 0x07)))); }
/* :898 SCTP_UNSET_TSN_PRESENT */
void sctp_unset(u8 *arry, u32 gap) { (arry[(gap >> 3)] &= ((~(0x01 << ((gap & 0x07)))) & 0xff)); }

/* ---- libsrtp (firefox third_party/libsrtp/src/crypto) ---------------------- */

/* include/datatypes.h:73 v128_t, include/rdb.h:62 srtp_rdb_t. */
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
#define v128_get_bit(x, bit) (((((x)->v32[(bit) >> 5]) >> ((bit)&31)) & 1))
#define v128_set_bit(x, bit) ((((x)->v32[(bit) >> 5]) |= ((u32)1 << ((bit)&31))))
#define v128_clear_bit(x, bit) ((((x)->v32[(bit) >> 5]) &= ~((u32)1 << ((bit)&31))))

/* include/datatypes.h:151 */
u32 srtp_get(const v128_t *x, u32 bit) { return v128_get_bit(x, bit); }
/* include/datatypes.h:153-154 */
void srtp_set(v128_t *x, u32 bit) { v128_set_bit(x, bit); }
/* include/datatypes.h:156-157 */
void srtp_clear(v128_t *x, u32 bit) { v128_clear_bit(x, bit); }
/* replay/rdb.c:74-93 (srtp_err_status_replay_fail = 9, _replay_old = 10). */
int srtp_rdb_check(const srtp_rdb_t *rdb, u32 p_index) {
    if (p_index >= rdb->window_start + (8 * sizeof(v128_t)))
        return 0;
    if (p_index < rdb->window_start)
        return 10;
    if (v128_get_bit(&rdb->bitmask, (p_index - rdb->window_start)) == 1)
        return 9;
    return 0;
}

/* ---- ICU (firefox intl/icu/source/common/unicode/utf8.h; chromium
 *      base/third_party/icu/icu_utf.h:149 is the same macro as CBU8_*) ------- */

/* utf8.h:98 U8_IS_VALID_LEAD3_AND_T1 */
int icu_lead3(int lead, u8 t1) {
    return ("\x20\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x10\x30\x30"[(lead)&0xf] &
            (1 << ((u8)(t1) >> 5))) != 0;
}
/* utf8.h:115 U8_IS_VALID_LEAD4_AND_T1 */
int icu_lead4(int lead, u8 t1) {
    return ("\x00\x00\x00\x00\x00\x00\x00\x00\x1E\x0F\x0F\x0F\x00\x00\x00\x00"[(u8)(t1) >> 4] &
            (1 << ((lead)&7))) != 0;
}

/* ---- brotli (firefox modules/brotli/dec/decode.c) -------------------------- */

/* :1240 */
void brotli_mark(u32 *trivial_literal_contexts, size_t i) {
    trivial_literal_contexts[i >> 5] |= 1u << (i & 31);
}
/* :1251-1252 */
u32 brotli_trivial(const u32 *trivial_literal_contexts, size_t block_type) {
    size_t trivial = trivial_literal_contexts[block_type >> 5];
    return (u32)((trivial >> (block_type & 31)) & 1);
}

/* ---- FreeType (firefox modules/freetype2/src) ------------------------------ */

/* sfnt/ttcmap.c:1865-1872, one iteration of the cmap8 is32 validation. */
int ft_is32_clear(const u8 *is32, u32 start) {
    u32 hi = start >> 16;
    u32 lo = start & 0xFFFFU;
    if ((is32[hi >> 3] & (0x80 >> (hi & 7))) == 0)
        return 1;
    if ((is32[lo >> 3] & (0x80 >> (lo & 7))) == 0)
        return 1;
    return 0;
}
/* sfnt/ttcmap.c:1884-1890, the start_hi == 0 range loop. */
int ft_is32_range_set(const u8 *is32, u32 start, u32 count) {
    for (; count > 0; count--, start++) {
        u32 lo = start & 0xFFFFU;
        if ((is32[lo >> 3] & (0x80 >> (lo & 7))) != 0)
            return 1;
    }
    return 0;
}
/* pshinter/pshrec.c:158 ps_mask_test_bit and its :712-714 caller. */
static int ps_mask_test_bit(const u8 *bytes, u32 idx) {
    return bytes[idx >> 3] & (0x80 >> (idx & 7));
}
int ft_mask_any3(const u8 *bytes, u32 hint1, u32 hint2, u32 hint3) {
    return ps_mask_test_bit(bytes, hint1) || ps_mask_test_bit(bytes, hint2) ||
           ps_mask_test_bit(bytes, hint3);
}
/* pshinter/pshrec.c:181-182 ps_mask_set_bit. */
void ft_mask_set(u8 *bytes, u32 idx) {
    u8 *p = bytes + (idx >> 3);
    p[0] = (u8)(p[0] | (0x80 >> (idx & 7)));
}

/* ---- NSS (firefox security/nss/lib/freebl/mpi/mplogic.c) ------------------- */

#define MP_DIGIT_BIT (8 * sizeof(unsigned long))
/* :342-354 mpl_set_bit, minus the s_mp_pad growth. */
void nss_set_bit(unsigned long *dp, u32 bitNum, u32 value) {
    u32 ix = bitNum / MP_DIGIT_BIT;
    unsigned long mask;
    bitNum = bitNum % MP_DIGIT_BIT;
    mask = (unsigned long)1 << bitNum;
    if (value)
        dp[ix] |= mask;
    else
        dp[ix] &= ~mask;
}
/* :372-377 mpl_get_bit, minus the range ARGCHK. */
int nss_get_bit(const unsigned long *dp, u32 bitNum) {
    u32 ix = bitNum / MP_DIGIT_BIT;
    u32 bit = bitNum % MP_DIGIT_BIT;
    return (int)(dp[ix] >> bit) & 1;
}

/* ---- libxml2 (chromium third_party/libxml/src/valid.c) --------------------- */

/* :293-299 _xmlValidState; :309 OCCURRENCE, :312 SET_OCCURRENCE; used at
 * :4500 `if (OCCURRENCE == 0)` and :4535 `SET_OCCURRENCE;`.  `1 << DEPTH` is
 * an int, sign-extended into the long: bit 31 is not a one-bit test. */
struct vstate {
    void *cont;
    void *node;
    long occurs;
    unsigned char depth;
    unsigned char state;
};
int libxml_occurs(struct vstate *v) {
    if ((v->occurs & (1 << v->depth)) == 0) {
        v->occurs |= (1 << v->depth);
        return 1;
    }
    return 0;
}

/* ---- LZMA SDK (chromium third_party/lzma_sdk/src/C) ------------------------ */

/* 7zArcIn.c:280-296 CountDefinedBits, verbatim. */
u32 lzma_count(const u8 *bits, u32 numItems) {
    unsigned b = 0;
    unsigned m = 0;
    u32 sum = 0;
    for (; numItems != 0; numItems--) {
        if (m == 0) {
            b = *bits++;
            m = 8;
        }
        m--;
        sum += (u32)((b >> m) & 1);
    }
    return sum;
}
/* 7zArcIn.c:338-345, the SzBitArray_Check (7z.h:75) gather loop. */
u32 lzma_defined_sum(const u8 *defs, u32 numItems, const u32 *data) {
    u32 i, sum = 0;
    for (i = 0; i < numItems; i++)
        if ((((defs)[(i) >> 3] & (0x80 >> ((i)&7))) != 0))
            sum += data[i];
    return sum;
}

/* browser_driver.c skips its own copies of the types above when included
 * after this file (browser_combined.c). */
#define BROWSER_BITS_TYPES 1
