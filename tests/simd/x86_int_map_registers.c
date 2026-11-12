/* Register homing of the packed integer/bitwise map families (RH-03).
 *
 * The pointer forms (`__builtin_ia32_paddb128`, `..._paddq256`, ...) take one
 * ADDRESS per operand and write their result through a `dest_ptr`, so a
 * register-resident operand is first spilled into a frame slot:
 *
 *     subq $88, %rsp
 *     movdqu %xmm1, 32(%rsp)          ; operand home
 *     vpaddb 32(%rsp), %xmm0, %xmm0   ; the same arithmetic, on memory
 *     addq $88, %rsp
 *
 * `try_native_packed_map` routes them to the register-valued twins the SLP
 * vectorizer already emits (`VecAddI8x16`, `VecCmpI32x8`, ...), which is
 * `vpaddb %xmm1, %xmm0, %xmm0` with no frame at all.  The counts below are
 * from this tree, `-O2 -mavx2 -mfma`, GCC 16.2 measured with the same
 * options and the same counting rule (instructions between the function label
 * and `ret`, `ret` included):
 *
 *     __m128i f(__m128i a, __m128i b){ return _mm_add_epi8(a,b); }    5 -> 2
 *     __m256i f(__m256i a, __m256i b){ return _mm256_add_epi8(a,b); } 9 -> 2
 *     _mm_and_si128 / _mm256_and_si256                                5/9 -> 2/2
 *     _mm_slli_epi32(a, 3) / _mm256_srai_epi16(a, 5)                  5/9 -> 2/2
 *     _mm_cmpeq_epi8 / _mm256_cmpgt_epi64                             5/9 -> 2/2
 *     _mm_add_epi8(_mm_loadu_si128(p), b)                             6 -> 3
 *     _mm_max_epi32 / _mm_max_epu32 / _mm_min_epu32                  48 -> 2
 *     _mm_max_epi8 / _mm_min_epi8                                 3 (c+b) -> 2
 *
 * The last two rows are not the pointer form's fault: the SSE4.1 extrema had
 * no builtin at all, so `include/smmintrin.h` computed them with a scalar loop
 * whose per-lane `cmov` chain the SLP vectorizer cannot fold back into
 * `pmaxsd`/`pmaxsb` (byte lanes reached 3 only because that loop's shape does
 * vectorize, as `pcmpgtb` + `pblendvb`).
 *
 * (The memory forms stop at 3 because the OTHER operand — a by-value vector
 * parameter — has a frame home of its own in this ABI, which is RH-04, not
 * this family: GCC keeps it in `%xmm0` and emits `vpaddb (%rdi), %xmm0,
 * %xmm0`.)
 *
 * The instruction count is what the gate budgets, but THIS FILE is the half
 * that says the rewrite computes the right function.  A lane map is written
 * by hand per builtin: `Paddb` is bytes and `Paddq` is qwords, both encode in
 * one instruction, and both are `vpadd*` — an instruction count cannot tell
 * them apart.  So every family is executed for every lane width at both
 * register widths against a scalar reference computed here, and the vector is
 * compared LANE BY LANE through its byte image.  The reference does not use
 * any intrinsic, so this is a differential test, not a self-check.
 *
 * Fallback coverage is part of the contract too: the twin table only takes
 * shift counts in the instruction's defined range (1..=15/31/63), so
 * `_mm_slli_epi32(a, 0)` (identity) and `_mm_slli_epi16(a, 17)` (all zeros)
 * keep the pointer path.  Both are checked here, because a "fast path" that
 * silently changed their meaning would be found by neither the budgets nor
 * the shapes it does handle.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int bad;

static void report(const char *what, int lane, uint64_t got, uint64_t want) {
    if (bad < 20) {
        printf("  %s lane %d: got %016llx want %016llx\n", what, lane,
               (unsigned long long)got, (unsigned long long)want);
    }
    bad++;
}

/* ---- the shapes the gate counts ---------------------------------------- */
/* Register-resident operands: the whole point of the routing is that these
   bodies contain no frame and no stack operand at all. */
__attribute__((noinline)) __m128i map_add8_128(__m128i a, __m128i b) {
    return _mm_add_epi8(a, b);
}

__attribute__((noinline)) __m256i map_add8_256(__m256i a, __m256i b) {
    return _mm256_add_epi8(a, b);
}

__attribute__((noinline)) __m128i map_and_128(__m128i a, __m128i b) {
    return _mm_and_si128(a, b);
}

__attribute__((noinline)) __m256i map_and_256(__m256i a, __m256i b) {
    return _mm256_and_si256(a, b);
}

__attribute__((noinline)) __m128i map_shift_128(__m128i a) {
    return _mm_slli_epi32(a, 3);
}

__attribute__((noinline)) __m256i map_shift_256(__m256i a) {
    return _mm256_srai_epi16(a, 5);
}

__attribute__((noinline)) __m128i map_cmp_128(__m128i a, __m128i b) {
    return _mm_cmpgt_epi8(a, b);
}

__attribute__((noinline)) __m256i map_cmp_256(__m256i a, __m256i b) {
    return _mm256_cmpgt_epi64(a, b);
}

__attribute__((noinline)) __m128i map_mul16_128(__m128i a, __m128i b) {
    return _mm_mullo_epi16(a, b);
}

__attribute__((noinline)) __m256i map_mul32_256(__m256i a, __m256i b) {
    return _mm256_mullo_epi32(a, b);
}

__attribute__((noinline)) __m128i map_min_128(__m128i a, __m128i b) {
    return _mm_min_epu8(a, b);
}

__attribute__((noinline)) __m256i map_max_256(__m256i a, __m256i b) {
    return _mm256_max_epi32(a, b);
}

/* SSE4.1 extrema.  Before RH-11's second half these six were 16- and 4-way
   scalar loops in `include/smmintrin.h`; `_mm_max_epi32` measured 48
   instructions and `_mm_min_epu32` 37 at -O2 against GCC's 2, because a
   per-lane `cmov` chain has no vector form the loop vectorizer recognizes.
   They are now the `pminsb`/`pmaxsb`/`pminsd`/`pmaxsd`/`pminud`/`pmaxud`
   encodings themselves, so each shape is one instruction plus `ret`. */
__attribute__((noinline)) __m128i map_min_epi8_128(__m128i a, __m128i b) {
    return _mm_min_epi8(a, b);
}

__attribute__((noinline)) __m128i map_max_epi8_128(__m128i a, __m128i b) {
    return _mm_max_epi8(a, b);
}

__attribute__((noinline)) __m128i map_min_epi32_128(__m128i a, __m128i b) {
    return _mm_min_epi32(a, b);
}

__attribute__((noinline)) __m128i map_max_epi32_128(__m128i a, __m128i b) {
    return _mm_max_epi32(a, b);
}

__attribute__((noinline)) __m128i map_min_epu32_128(__m128i a, __m128i b) {
    return _mm_min_epu32(a, b);
}

__attribute__((noinline)) __m128i map_max_epu32_128(__m128i a, __m128i b) {
    return _mm_max_epu32(a, b);
}

/* The four AVX2 spellings that did not exist at all: `-mavx2` code that GCC
   compiles failed here with "implicit declaration of function". */
__attribute__((noinline)) __m256i map_min_epi8_256(__m256i a, __m256i b) {
    return _mm256_min_epi8(a, b);
}

__attribute__((noinline)) __m256i map_max_epi8_256(__m256i a, __m256i b) {
    return _mm256_max_epi8(a, b);
}

__attribute__((noinline)) __m256i map_min_epu32_256(__m256i a, __m256i b) {
    return _mm256_min_epu32(a, b);
}

__attribute__((noinline)) __m256i map_max_epu32_256(__m256i a, __m256i b) {
    return _mm256_max_epu32(a, b);
}

/* Memory-operand shapes: one operand arrives from memory.  The other is a
   by-value vector parameter, whose frame home is an ABI fact (RH-04), so the
   budget is one instruction above the register shapes. */
__attribute__((noinline)) __m128i map_add8m_128(const void *p, __m128i b) {
    return _mm_add_epi8(_mm_loadu_si128((const __m128i *)p), b);
}

/* Same shape for the new extrema: the folded load proves the pointer form's
   three-address ABI is not what carries the value here. */
__attribute__((noinline)) __m128i map_maxsd32m_128(const void *p, __m128i b) {
    return _mm_max_epi32(_mm_loadu_si128((const __m128i *)p), b);
}

__attribute__((noinline)) __m256i map_add64m_256(const void *p, __m256i b) {
    return _mm256_add_epi64(_mm256_loadu_si256((const __m256i *)p), b);
}

/* ---- inlined wrappers: by-value vector parameters ---------------------- */
/* Everything SSE4.1 and later in the headers is a `target(...)`-attributed
   always-inline function, so using it means INLINING a callee whose two
   16-byte vector parameters are by value.  The inliner bridges such a
   parameter with a `memcpy` into a fresh home; `forward_memcpy_filled_reads`
   (passes/aggregate_copy_forward.rs) forwards the reads to the caller's home,
   which the backend already keeps in the incoming %xmm/%ymm.  Without that
   forwarding each wrapper paid a 16-byte store/load pair per parameter
   (5 instructions, or 8-12 with a frame) for one packed instruction.
   These four shapes pin the property; the counterfactual below runs them
   with the forwarding switched off. */
static __inline__ __m128i
    __attribute__((__always_inline__, target("sse4.1")))
    wrap_add16(__m128i a, __m128i b) {
    return _mm_add_epi16(a, b);
}

static __inline__ __m128i
    __attribute__((__always_inline__, target("sse4.1")))
    wrap_maxu16(__m128i a, __m128i b) {
    return _mm_max_epu16(a, b);
}

static __inline__ __m256i
    __attribute__((__always_inline__, target("avx2")))
    wrap_minu16_256(__m256i a, __m256i b) {
    return _mm256_min_epu16(a, b);
}

static __inline__ __m256i
    __attribute__((__always_inline__, target("avx2")))
    wrap_and256(__m256i a, __m256i b) {
    return _mm256_and_si256(a, b);
}

__attribute__((noinline)) __m128i map_wrap_add16_128(__m128i a, __m128i b) {
    return wrap_add16(a, b);
}

__attribute__((noinline)) __m128i map_wrap_maxu16_128(__m128i a, __m128i b) {
    return wrap_maxu16(a, b);
}

__attribute__((noinline)) __m256i map_wrap_minu16_256(__m256i a, __m256i b) {
    return wrap_minu16_256(a, b);
}

__attribute__((noinline)) __m256i map_wrap_and256_256(__m256i a, __m256i b) {
    return wrap_and256(a, b);
}

/* ---- scalar reference -------------------------------------------------- */

enum refop {
    REF_ADD,
    REF_SUB,
    REF_AND,
    REF_OR,
    REF_XOR,
    REF_MINU,
    REF_MAXU,
    REF_MINS,
    REF_MAXS,
    REF_MULLO
};

static uint64_t lane_mask(int bits) {
    return bits == 64 ? ~(uint64_t)0 : ((uint64_t)1 << bits) - 1;
}

static int64_t sext(uint64_t v, int bits) {
    if (bits == 64) {
        return (int64_t)v;
    }
    uint64_t sign = (uint64_t)1 << (bits - 1);
    return (int64_t)((v ^ sign) - sign);
}

static uint64_t ref_eval(int op, int bits, uint64_t a, uint64_t b) {
    uint64_t m = lane_mask(bits);
    a &= m;
    b &= m;
    switch (op) {
    case REF_ADD:
        return (a + b) & m;
    case REF_SUB:
        return (a - b) & m;
    case REF_AND:
        return a & b;
    case REF_OR:
        return a | b;
    case REF_XOR:
        return a ^ b;
    case REF_MINU:
        return a < b ? a : b;
    case REF_MAXU:
        return a > b ? a : b;
    case REF_MINS:
        return sext(a, bits) < sext(b, bits) ? a : b;
    case REF_MAXS:
        return sext(a, bits) > sext(b, bits) ? a : b;
    case REF_MULLO:
        return (a * b) & m;
    default:
        return 0;
    }
}

static uint64_t load_lane(const unsigned char *p, int bytes) {
    uint64_t v = 0;
    for (int i = 0; i < bytes; i++) {
        v |= (uint64_t)p[i] << (8 * i);
    }
    return v;
}

static void check_map(const char *what, const unsigned char *got,
                      const unsigned char *a, const unsigned char *b,
                      int bytes, int lanes, int op) {
    for (int i = 0; i < lanes; i++) {
        uint64_t av = load_lane(a + i * bytes, bytes);
        uint64_t bv = load_lane(b + i * bytes, bytes);
        uint64_t want = ref_eval(op, bytes * 8, av, bv);
        uint64_t gv = load_lane(got + i * bytes, bytes);
        if (gv != want) {
            report(what, i, gv, want);
        }
    }
}

/* Intel defines a logical shift of a lane by MORE than its width as zero, and
   an arithmetic shift by more than its width as a full sign fill; a shift by
   zero is the identity.  Only 1..=width-1 reaches the register twin, so the
   other three cases are checked against THIS reference. */
static uint64_t ref_shift(int arithmetic, int right, int bits, uint64_t v,
                          int count) {
    uint64_t m = lane_mask(bits);
    v &= m;
    if (count == 0) {
        return v;
    }
    if (count >= bits) {
        if (!right || !arithmetic) {
            return 0;
        }
        return sext(v, bits) < 0 ? m : 0;
    }
    if (!right) {
        return (v << count) & m;
    }
    if (!arithmetic) {
        return v >> count;
    }
    return (uint64_t)(sext(v, bits) >> count) & m;
}

static void check_shift(const char *what, const unsigned char *got,
                        const unsigned char *a, int bytes, int lanes,
                        int arithmetic, int right, int count) {
    for (int i = 0; i < lanes; i++) {
        uint64_t av = load_lane(a + i * bytes, bytes);
        uint64_t want = ref_shift(arithmetic, right, bytes * 8, av, count);
        uint64_t gv = load_lane(got + i * bytes, bytes);
        if (gv != want) {
            report(what, i, gv, want);
        }
    }
}

/* `pcmp{eq,gt}X` produce an all-ones lane where the comparison holds. */
static void check_cmp(const char *what, const unsigned char *got,
                      const unsigned char *a, const unsigned char *b,
                      int bytes, int lanes, int greater) {
    for (int i = 0; i < lanes; i++) {
        uint64_t av = load_lane(a + i * bytes, bytes);
        uint64_t bv = load_lane(b + i * bytes, bytes);
        int64_t sa = sext(av, bytes * 8);
        int64_t sb = sext(bv, bytes * 8);
        int truth = greater ? (sa > sb) : (av == bv);
        uint64_t want = truth ? lane_mask(bytes * 8) : 0;
        uint64_t gv = load_lane(got + i * bytes, bytes);
        if (gv != want) {
            report(what, i, gv, want);
        }
    }
}

/* ---- call sites -------------------------------------------------------- */

#define RUN_MAP128(NAME, FN, OP, BYTES, LANES)                                 \
    do {                                                                       \
        __m128i va, vb, r;                                                     \
        memcpy(&va, A128, 16);                                                 \
        memcpy(&vb, B128, 16);                                                 \
        r = FN(va, vb);                                                        \
        check_map(NAME, (const unsigned char *)&r, A128, B128, BYTES, LANES,   \
                  OP);                                                         \
    } while (0)

/* Memory-operand form: operand A comes from the array, operand B by value.
   Kept a separate macro because the shape under test takes a POINTER first --
   passing the vector value would test the wrong function entirely. */
#define RUN_MAPM128(NAME, FN, OP, BYTES, LANES)                                \
    do {                                                                       \
        __m128i vb, r;                                                         \
        memcpy(&vb, B128, 16);                                                 \
        r = FN(A128, vb);                                                      \
        check_map(NAME, (const unsigned char *)&r, A128, B128, BYTES, LANES,   \
                  OP);                                                         \
    } while (0)

#define RUN_MAP256(NAME, FN, OP, BYTES, LANES)                                 \
    do {                                                                       \
        __m256i va, vb, r;                                                     \
        memcpy(&va, A256, 32);                                                 \
        memcpy(&vb, B256, 32);                                                 \
        r = FN(va, vb);                                                        \
        check_map(NAME, (const unsigned char *)&r, A256, B256, BYTES, LANES,   \
                  OP);                                                         \
    } while (0)

#define RUN_SHIFT128(NAME, FN, BYTES, LANES, ARITH, RIGHT, COUNT)              \
    do {                                                                       \
        __m128i va, r;                                                         \
        memcpy(&va, A128, 16);                                                 \
        r = FN(va, (COUNT));                                                   \
        check_shift(NAME, (const unsigned char *)&r, A128, BYTES, LANES, ARITH, \
                    RIGHT, (COUNT));                                           \
    } while (0)

#define RUN_SHIFT256(NAME, FN, BYTES, LANES, ARITH, RIGHT, COUNT)              \
    do {                                                                       \
        __m256i va, r;                                                         \
        memcpy(&va, A256, 32);                                                 \
        r = FN(va, (COUNT));                                                   \
        check_shift(NAME, (const unsigned char *)&r, A256, BYTES, LANES, ARITH, \
                    RIGHT, (COUNT));                                           \
    } while (0)

#define RUN_CMP128(NAME, FN, BYTES, LANES, GREATER)                            \
    do {                                                                       \
        __m128i va, vb, r;                                                     \
        memcpy(&va, A128, 16);                                                 \
        memcpy(&vb, B128, 16);                                                 \
        r = FN(va, vb);                                                        \
        check_cmp(NAME, (const unsigned char *)&r, A128, B128, BYTES, LANES,   \
                  GREATER);                                                    \
    } while (0)

#define RUN_CMP256(NAME, FN, BYTES, LANES, GREATER)                            \
    do {                                                                       \
        __m256i va, vb, r;                                                     \
        memcpy(&va, A256, 32);                                                 \
        memcpy(&vb, B256, 32);                                                 \
        r = FN(va, vb);                                                        \
        check_cmp(NAME, (const unsigned char *)&r, A256, B256, BYTES, LANES,   \
                  GREATER);                                                    \
    } while (0)

/* ---- operands ---------------------------------------------------------- */

/* A pattern that is NOT self-similar across lanes: it carries zeros, ones,
   the sign bit of every lane width, all-ones, and values that differ between
   the two operands in the sign as well as in the low bits, so a swapped or
   width-mismatched lane map cannot accidentally agree. */
static void fill(unsigned char *p, int n, int salt) {
    for (int i = 0; i < n; i++) {
        p[i] = (unsigned char)((i * 37 + salt * 101 + (i >> 3) * 7) & 0xff);
    }
    /* Explicit corners, laid over the pattern. */
    p[0] = 0x00;
    p[1] = 0x80;
    p[2] = 0xff;
    p[3] = 0x01;
    if (n > 8) {
        p[8] = 0x7f;
        p[9] = 0x81;
    }
}

/* Sign-disagreement corners for the signed/unsigned extrema families.
 *
 * A min/max family that reads its lanes with the WRONG signedness computes
 * the same function whenever both operands of a lane share a sign: `pminsb`
 * and `pminub` differ only where exactly one operand's high bit is set.  The
 * generic pattern above does not guarantee such a lane at every width (its
 * high bytes happen to agree in sign at most lanes), which would let a
 * signedness bug through while every value still matched.
 *
 * Rows 1/2 hold the low/high byte of each operand's lane-0..3 words and
 * dwords; `sign_corners` sets, for each lane width, ONE lane where the two
 * operands disagree in sign -- the positive operand in `lo`, the negative one
 * in `hi`.  One lane is enough to separate signed from unsigned for both
 * min and max (in that lane the signed answer is `lo` for max and `hi` for
 * min; the unsigned answer is the opposite of both).  The positions are
 * disjoint so the four width requirements hold simultaneously:
 *
 *   width 1 -- byte  5  (lane 5)
 *   width 2 -- byte  1  (word 0 high byte)
 *   width 4 -- byte  3  (dword 0 high byte)
 *   width 8 -- byte  7  (qword 0 high byte)
 */
static void sign_corners(unsigned char *lo, int lo_n, unsigned char *hi,
                         int hi_n) {
    static const int pos[] = {5, 1, 3, 7};
    for (unsigned i = 0; i < sizeof(pos) / sizeof(pos[0]); i++) {
        if (pos[i] < lo_n) {
            lo[pos[i]] = 0x10;
        }
        if (pos[i] < hi_n) {
            hi[pos[i]] = 0x90;
        }
    }
}

int main(void) {
    unsigned char A128[16], B128[16], A256[32], B256[32];
    fill(A128, 16, 1);
    fill(B128, 16, 2);
    fill(A256, 32, 3);
    fill(B256, 32, 4);
    /* Every lane width gets a lane where the operands disagree in sign, so a
       signed family lowered as unsigned (or the reverse) cannot pass. */
    sign_corners(A128, 16, B128, 16);
    sign_corners(A256, 32, B256, 32);

    /* ---- arithmetic and bitwise, every lane width, both widths -------- */
    RUN_MAP128("add_epi8", _mm_add_epi8, REF_ADD, 1, 16);
    RUN_MAP128("add_epi16", _mm_add_epi16, REF_ADD, 2, 8);
    RUN_MAP128("add_epi32", _mm_add_epi32, REF_ADD, 4, 4);
    RUN_MAP128("add_epi64", _mm_add_epi64, REF_ADD, 8, 2);
    RUN_MAP128("sub_epi8", _mm_sub_epi8, REF_SUB, 1, 16);
    RUN_MAP128("sub_epi16", _mm_sub_epi16, REF_SUB, 2, 8);
    RUN_MAP128("sub_epi32", _mm_sub_epi32, REF_SUB, 4, 4);
    RUN_MAP128("sub_epi64", _mm_sub_epi64, REF_SUB, 8, 2);
    RUN_MAP128("and_si128", _mm_and_si128, REF_AND, 4, 4);
    RUN_MAP128("or_si128", _mm_or_si128, REF_OR, 4, 4);
    RUN_MAP128("xor_si128", _mm_xor_si128, REF_XOR, 4, 4);
    RUN_MAP128("mullo_epi16", _mm_mullo_epi16, REF_MULLO, 2, 8);
    RUN_MAP128("mullo_epi32", _mm_mullo_epi32, REF_MULLO, 4, 4);
    RUN_MAP128("min_epu8", _mm_min_epu8, REF_MINU, 1, 16);
    RUN_MAP128("max_epu8", _mm_max_epu8, REF_MAXU, 1, 16);
    RUN_MAP128("min_epi16", _mm_min_epi16, REF_MINS, 2, 8);
    RUN_MAP128("max_epi16", _mm_max_epi16, REF_MAXS, 2, 8);
    /* The unsigned 16-bit extrema have no `pminuw`/`pmaxuw` value form: the
       headers express them as exact saturating-difference identities, and
       the values still have to be right for every lane. */
    RUN_MAP128("min_epu16", _mm_min_epu16, REF_MINU, 2, 8);
    RUN_MAP128("max_epu16", _mm_max_epu16, REF_MAXU, 2, 8);
    RUN_MAP128("wrap_add16", wrap_add16, REF_ADD, 2, 8);
    RUN_MAP128("wrap_maxu16", wrap_maxu16, REF_MAXU, 2, 8);
    /* SSE4.1 extrema: the four sign structures at the two new lane widths. */
    RUN_MAP128("min_epi8_128", _mm_min_epi8, REF_MINS, 1, 16);
    RUN_MAP128("max_epi8_128", _mm_max_epi8, REF_MAXS, 1, 16);
    RUN_MAP128("min_epi32_128", _mm_min_epi32, REF_MINS, 4, 4);
    RUN_MAP128("max_epi32_128", _mm_max_epi32, REF_MAXS, 4, 4);
    RUN_MAP128("min_epu32_128", _mm_min_epu32, REF_MINU, 4, 4);
    RUN_MAP128("max_epu32_128", _mm_max_epu32, REF_MAXU, 4, 4);
    /* Memory-operand form: the folded load must not change the answer. */
    RUN_MAPM128("maxsd32m_128", map_maxsd32m_128, REF_MAXS, 4, 4);

    RUN_MAP256("add_epi8_256", _mm256_add_epi8, REF_ADD, 1, 32);
    RUN_MAP256("add_epi16_256", _mm256_add_epi16, REF_ADD, 2, 16);
    RUN_MAP256("add_epi32_256", _mm256_add_epi32, REF_ADD, 4, 8);
    RUN_MAP256("add_epi64_256", _mm256_add_epi64, REF_ADD, 8, 4);
    RUN_MAP256("sub_epi8_256", _mm256_sub_epi8, REF_SUB, 1, 32);
    RUN_MAP256("sub_epi16_256", _mm256_sub_epi16, REF_SUB, 2, 16);
    RUN_MAP256("sub_epi32_256", _mm256_sub_epi32, REF_SUB, 4, 8);
    RUN_MAP256("sub_epi64_256", _mm256_sub_epi64, REF_SUB, 8, 4);
    RUN_MAP256("and_si256", _mm256_and_si256, REF_AND, 4, 8);
    RUN_MAP256("or_si256", _mm256_or_si256, REF_OR, 4, 8);
    RUN_MAP256("xor_si256", _mm256_xor_si256, REF_XOR, 4, 8);
    RUN_MAP256("mullo_epi16_256", _mm256_mullo_epi16, REF_MULLO, 2, 16);
    RUN_MAP256("mullo_epi32_256", _mm256_mullo_epi32, REF_MULLO, 4, 8);
    RUN_MAP256("min_epu8_256", _mm256_min_epu8, REF_MINU, 1, 32);
    RUN_MAP256("max_epu8_256", _mm256_max_epu8, REF_MAXU, 1, 32);
    RUN_MAP256("min_epi32_256", _mm256_min_epi32, REF_MINS, 4, 8);
    RUN_MAP256("max_epi32_256", _mm256_max_epi32, REF_MAXS, 4, 8);
    RUN_MAP256("min_epu16_256", _mm256_min_epu16, REF_MINU, 2, 16);
    RUN_MAP256("max_epu16_256", _mm256_max_epu16, REF_MAXU, 2, 16);
    RUN_MAP256("wrap_minu16_256", wrap_minu16_256, REF_MINU, 2, 16);
    RUN_MAP256("wrap_and256", wrap_and256, REF_AND, 4, 8);
    RUN_MAP256("min_epi8_256", _mm256_min_epi8, REF_MINS, 1, 32);
    RUN_MAP256("max_epi8_256", _mm256_max_epi8, REF_MAXS, 1, 32);
    RUN_MAP256("min_epu32_256", _mm256_min_epu32, REF_MINU, 4, 8);
    RUN_MAP256("max_epu32_256", _mm256_max_epu32, REF_MAXU, 4, 8);
    RUN_MAP256("min_epi32_256", _mm256_min_epi32, REF_MINS, 4, 8);

    /* ---- immediate shifts -------------------------------------------- */
    RUN_SHIFT128("slli_epi16_3", _mm_slli_epi16, 2, 8, 0, 0, 3);
    RUN_SHIFT128("slli_epi32_3", _mm_slli_epi32, 4, 4, 0, 0, 3);
    RUN_SHIFT128("slli_epi64_3", _mm_slli_epi64, 8, 2, 0, 0, 3);
    RUN_SHIFT128("srli_epi16_3", _mm_srli_epi16, 2, 8, 0, 1, 3);
    RUN_SHIFT128("srli_epi32_3", _mm_srli_epi32, 4, 4, 0, 1, 3);
    RUN_SHIFT128("srli_epi64_3", _mm_srli_epi64, 8, 2, 0, 1, 3);
    RUN_SHIFT128("srai_epi16_3", _mm_srai_epi16, 2, 8, 1, 1, 3);
    RUN_SHIFT128("srai_epi32_3", _mm_srai_epi32, 4, 4, 1, 1, 3);
    /* boundaries: the largest in-range count, then the fallback shapes */
    RUN_SHIFT128("slli_epi16_15", _mm_slli_epi16, 2, 8, 0, 0, 15);
    RUN_SHIFT128("slli_epi64_63", _mm_slli_epi64, 8, 2, 0, 0, 63);
    RUN_SHIFT128("srli_epi64_63", _mm_srli_epi64, 8, 2, 0, 1, 63);
    RUN_SHIFT128("ara_boundary_31", _mm_srai_epi32, 4, 4, 1, 1, 31);
    RUN_SHIFT128("slli_epi32_0", _mm_slli_epi32, 4, 4, 0, 0, 0);
    RUN_SHIFT128("srli_epi32_0", _mm_srli_epi32, 4, 4, 0, 1, 0);
    RUN_SHIFT128("slli_epi16_17", _mm_slli_epi16, 2, 8, 0, 0, 17);
    RUN_SHIFT128("srli_epi16_17", _mm_srli_epi16, 2, 8, 0, 1, 17);
    RUN_SHIFT128("srai_epi16_17", _mm_srai_epi16, 2, 8, 1, 1, 17);
    RUN_SHIFT128("slli_epi32_33", _mm_slli_epi32, 4, 4, 0, 0, 33);
    RUN_SHIFT128("srai_epi32_33", _mm_srai_epi32, 4, 4, 1, 1, 33);

    RUN_SHIFT256("slli_epi16_3_256", _mm256_slli_epi16, 2, 16, 0, 0, 3);
    RUN_SHIFT256("slli_epi32_3_256", _mm256_slli_epi32, 4, 8, 0, 0, 3);
    RUN_SHIFT256("slli_epi64_3_256", _mm256_slli_epi64, 8, 4, 0, 0, 3);
    RUN_SHIFT256("srli_epi16_3_256", _mm256_srli_epi16, 2, 16, 0, 1, 3);
    RUN_SHIFT256("srli_epi32_3_256", _mm256_srli_epi32, 4, 8, 0, 1, 3);
    RUN_SHIFT256("srli_epi64_3_256", _mm256_srli_epi64, 8, 4, 0, 1, 3);
    RUN_SHIFT256("srai_epi16_3_256", _mm256_srai_epi16, 2, 16, 1, 1, 3);
    RUN_SHIFT256("srai_epi32_3_256", _mm256_srai_epi32, 4, 8, 1, 1, 3);
    RUN_SHIFT256("slli_epi32_0_256", _mm256_slli_epi32, 4, 8, 0, 0, 0);
    RUN_SHIFT256("srai_epi16_20_256", _mm256_srai_epi16, 2, 16, 1, 1, 20);
    RUN_SHIFT256("slli_epi16_40_256", _mm256_slli_epi16, 2, 16, 0, 0, 40);

    /* ---- compares ---------------------------------------------------- */
    RUN_CMP128("cmpeq_epi8", _mm_cmpeq_epi8, 1, 16, 0);
    RUN_CMP128("cmpeq_epi16", _mm_cmpeq_epi16, 2, 8, 0);
    RUN_CMP128("cmpeq_epi32", _mm_cmpeq_epi32, 4, 4, 0);
    RUN_CMP128("cmpgt_epi8", _mm_cmpgt_epi8, 1, 16, 1);
    RUN_CMP128("cmpgt_epi16", _mm_cmpgt_epi16, 2, 8, 1);
    RUN_CMP128("cmpgt_epi32", _mm_cmpgt_epi32, 4, 4, 1);
    RUN_CMP256("cmpeq_epi8_256", _mm256_cmpeq_epi8, 1, 32, 0);
    RUN_CMP256("cmpeq_epi32_256", _mm256_cmpeq_epi32, 4, 8, 0);
    RUN_CMP256("cmpeq_epi64_256", _mm256_cmpeq_epi64, 8, 4, 0);
    RUN_CMP256("cmpgt_epi8_256", _mm256_cmpgt_epi8, 1, 32, 1);
    RUN_CMP256("cmpgt_epi32_256", _mm256_cmpgt_epi32, 4, 8, 1);
    RUN_CMP256("cmpgt_epi64_256", _mm256_cmpgt_epi64, 8, 4, 1);

    /* ---- float bitwise (same instructions, float register image) ----- */
    {
        __m128 va, vb, r;
        memcpy(&va, A128, 16);
        memcpy(&vb, B128, 16);
        r = _mm_and_ps(va, vb);
        check_map("and_ps", (const unsigned char *)&r, A128, B128, 4, 4, REF_AND);
        r = _mm_or_ps(va, vb);
        check_map("or_ps", (const unsigned char *)&r, A128, B128, 4, 4, REF_OR);
        r = _mm_xor_ps(va, vb);
        check_map("xor_ps", (const unsigned char *)&r, A128, B128, 4, 4, REF_XOR);
    }

    if (bad) {
        printf("FAIL: %d lane(s) disagree with the scalar reference\n", bad);
        return 1;
    }
    printf("PASS: every packed map family agrees with the scalar reference "
           "for every lane width at 128 and 256 bits\n");
    return 0;
}
