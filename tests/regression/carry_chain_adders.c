/* Full adders: `x + y + carry` and `x - y - borrow` with their carry out,
 * in every spelling C has for them -- the header intrinsics
 * (`_addcarry_u64`, `_subborrow_u32`, ...), the builtins
 * (`__builtin_addcll`, `__builtin_subc`, ...), comparisons
 * (`s = a + b; c = s < a; t = s + cin; c |= t < s`), `unsigned __int128`
 * and the limb loops of GMP, mini-gmp and OpenSSL as they are written
 * there -- alone, in chains of limbs, in loops, across branches, with
 * constant operands, with a carry in that is a whole word, and with either
 * result unused.
 *
 * On x86-64 the optimizer turns each of them into `adc`/`sbb`
 * (src/passes/carry_chain.rs). Every result is checked here against an
 * adder that computes in 32-bit halves, and printed, so that the output of
 * the reference compiler is a second witness. */
#include <stdint.h>
#include <stdio.h>

#if defined(__x86_64__)
#include <immintrin.h>
#define HAVE_HEADER 1
#else
#define HAVE_HEADER 0
#endif

typedef uint64_t u64;
typedef uint32_t u32;
typedef uint8_t u8;

#define NOINLINE __attribute__((noinline))

static u64 checksum = 0xcbf29ce484222325ull;
static unsigned long failures;

static void mix(u64 v)
{
    checksum = (checksum ^ v) * 0x100000001b3ull;
    checksum ^= checksum >> 29;
}

/* ---- The reference: no comparison, no `|` of comparisons. ---- */

static NOINLINE u64 ref_add64(u64 a, u64 b, unsigned c, unsigned *out)
{
    u64 lo = (a & 0xffffffffu) + (b & 0xffffffffu) + (c != 0);
    u64 hi = (a >> 32) + (b >> 32) + (lo >> 32);
    *out = (unsigned)(hi >> 32);
    return (hi << 32) | (lo & 0xffffffffu);
}

static NOINLINE u64 ref_sub64(u64 a, u64 b, unsigned c, unsigned *out)
{
    /* a - b - c == a + ~b + !c, and the borrow is the missing carry. */
    unsigned carry;
    u64 r = ref_add64(a, ~b, c == 0, &carry);
    *out = !carry;
    return r;
}

static NOINLINE u32 ref_add32(u32 a, u32 b, unsigned c, unsigned *out)
{
    u64 wide = (u64)a + b + (c != 0);
    *out = (unsigned)(wide >> 32);
    return (u32)wide;
}

static NOINLINE u32 ref_sub32(u32 a, u32 b, unsigned c, unsigned *out)
{
    u64 wide = (u64)a - b - (c != 0);
    *out = (unsigned)(wide >> 63);
    return (u32)wide;
}

static void expect(const char *what, u64 got, u64 want)
{
    mix(got);
    if (got != want && failures++ < 20)
        printf("FAIL %s: got %llx want %llx\n", what, (unsigned long long)got,
               (unsigned long long)want);
}

/* ---- Inputs. ---- */

static const u64 edges[] = {
    0, 1, 2, 0x7fffffffull, 0x80000000ull, 0xfffffffeull, 0xffffffffull,
    0x100000000ull, 0x7fffffffffffffffull, 0x8000000000000000ull,
    0xfffffffffffffffeull, 0xffffffffffffffffull, 0xffffffff00000000ull,
    0x00000000ffffffffull + 0x100000000ull,
};
#define NEDGES (sizeof edges / sizeof edges[0])

static u64 rng_state = 0x9e3779b97f4a7c15ull;
static u64 rng(void)
{
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 7;
    rng_state ^= rng_state << 17;
    return rng_state;
}

/* A limb that is an edge more often than not, so that carries ripple. */
static u64 limb(void)
{
    u64 r = rng();
    return (r & 3) ? edges[(r >> 8) % NEDGES] : rng();
}

/* What a carry in may hold: an `unsigned char` that is not a bit. */
static const u8 carries[] = {0, 1, 2, 0x80, 0xff};
#define NCARRIES (sizeof carries / sizeof carries[0])

/* ---- 1. The header intrinsics, one adder each. ---- */

#if HAVE_HEADER
static NOINLINE u8 hdr_add64(u8 c, u64 a, u64 b, u64 *r)
{
    return _addcarry_u64(c, a, b, (unsigned long long *)r);
}

static NOINLINE u8 hdr_sub64(u8 c, u64 a, u64 b, u64 *r)
{
    return _subborrow_u64(c, a, b, (unsigned long long *)r);
}

static NOINLINE u8 hdr_add32(u8 c, u32 a, u32 b, u32 *r)
{
    return _addcarry_u32(c, a, b, r);
}

static NOINLINE u8 hdr_sub32(u8 c, u32 a, u32 b, u32 *r)
{
    return _subborrow_u32(c, a, b, r);
}

/* The carry alone, and the word alone. */
static NOINLINE u8 hdr_carry_only(u8 c, u64 a, u64 b)
{
    unsigned long long unused;
    return _addcarry_u64(c, a, b, &unused);
}

static NOINLINE u64 hdr_word_only(u8 c, u64 a, u64 b)
{
    unsigned long long r;
    (void)_subborrow_u64(c, a, b, &r);
    return r;
}

/* A 32-bit adder whose results are read at 64 bits: the word must arrive
 * zero-extended, the carry as 0 or 1. */
static NOINLINE u64 hdr_add32_wide(u8 c, u32 a, u32 b)
{
    u32 r;
    u8 o = _addcarry_u32(c, a, b, &r);
    return ((u64)o << 40) ^ (u64)r ^ ((u64)r << 3);
}

/* Constant operands: an immediate, a sign-extended one, one that does not
 * fit an immediate, a constant carry, the same operand twice, a zero. */
static NOINLINE u8 hdr_add_one(u8 c, u64 a, u64 *r)
{
    return _addcarry_u64(c, a, 1, (unsigned long long *)r);
}

static NOINLINE u8 hdr_add_minus_one(u8 c, u64 a, u64 *r)
{
    return _addcarry_u64(c, a, ~0ull, (unsigned long long *)r);
}

static NOINLINE u8 hdr_sub_big(u8 c, u64 a, u64 *r)
{
    return _subborrow_u64(c, a, 0x123456789ull, (unsigned long long *)r);
}

static NOINLINE u8 hdr_add32_all_ones(u8 c, u32 a, u32 *r)
{
    return _addcarry_u32(c, a, 0xffffffffu, r);
}

static NOINLINE u8 hdr_add_carry_set(u64 a, u64 b, u64 *r)
{
    return _addcarry_u64(1, a, b, (unsigned long long *)r);
}

static NOINLINE u8 hdr_sub_borrow_set(u64 a, u64 b, u64 *r)
{
    return _subborrow_u64(1, a, b, (unsigned long long *)r);
}

static NOINLINE u8 hdr_double(u8 c, u64 a, u64 *r)
{
    return _addcarry_u64(c, a, a, (unsigned long long *)r);
}

static NOINLINE u8 hdr_negate(u8 c, u64 a, u64 *r)
{
    return _subborrow_u64(c, 0, a, (unsigned long long *)r);
}

static NOINLINE u8 hdr_from_constant(u8 c, u64 b, u64 *r)
{
    return _addcarry_u64(c, 0xfedcba9876543210ull, b, (unsigned long long *)r);
}

/* ---- 2. Chains of them. ---- */

static NOINLINE void hdr_add4(u64 *r, const u64 *a, const u64 *b)
{
    u8 c = _addcarry_u64(0, a[0], b[0], (unsigned long long *)&r[0]);
    c = _addcarry_u64(c, a[1], b[1], (unsigned long long *)&r[1]);
    c = _addcarry_u64(c, a[2], b[2], (unsigned long long *)&r[2]);
    (void)_addcarry_u64(c, a[3], b[3], (unsigned long long *)&r[3]);
}

static NOINLINE u8 hdr_sub4(u64 *r, const u64 *a, const u64 *b)
{
    u8 c = _subborrow_u64(0, a[0], b[0], (unsigned long long *)&r[0]);
    c = _subborrow_u64(c, a[1], b[1], (unsigned long long *)&r[1]);
    c = _subborrow_u64(c, a[2], b[2], (unsigned long long *)&r[2]);
    return _subborrow_u64(c, a[3], b[3], (unsigned long long *)&r[3]);
}

static NOINLINE u8 hdr_add3_carry_in(u8 c, u64 *r, const u64 *a, const u64 *b)
{
    c = _addcarry_u64(c, a[0], b[0], (unsigned long long *)&r[0]);
    c = _addcarry_u64(c, a[1], b[1], (unsigned long long *)&r[1]);
    return _addcarry_u64(c, a[2], b[2], (unsigned long long *)&r[2]);
}

static NOINLINE u8 hdr_add4_32(u32 *r, const u32 *a, const u32 *b)
{
    u8 c = _addcarry_u32(0, a[0], b[0], &r[0]);
    c = _addcarry_u32(c, a[1], b[1], &r[1]);
    c = _addcarry_u32(c, a[2], b[2], &r[2]);
    return _addcarry_u32(c, a[3], b[3], &r[3]);
}

/* Every carry of the chain is also counted: the bit must exist. */
static NOINLINE unsigned hdr_add4_counted(u64 *r, const u64 *a, const u64 *b)
{
    unsigned n = 0;
    u8 c = _addcarry_u64(0, a[0], b[0], (unsigned long long *)&r[0]);
    n += c;
    c = _addcarry_u64(c, a[1], b[1], (unsigned long long *)&r[1]);
    n += c;
    c = _addcarry_u64(c, a[2], b[2], (unsigned long long *)&r[2]);
    n += c;
    c = _addcarry_u64(c, a[3], b[3], (unsigned long long *)&r[3]);
    return n + c;
}

/* Additions and subtractions in one chain: the carry of one is the borrow
 * of the next. */
static NOINLINE u8 hdr_mixed(u64 *r, const u64 *a, const u64 *b)
{
    u8 c = _addcarry_u64(0, a[0], b[0], (unsigned long long *)&r[0]);
    c = _subborrow_u64(c, a[1], b[1], (unsigned long long *)&r[1]);
    c = _addcarry_u64(c, a[2], b[2], (unsigned long long *)&r[2]);
    return _subborrow_u64(c, a[3], b[3], (unsigned long long *)&r[3]);
}

/* Something that writes the flags between two links. */
static NOINLINE u8 hdr_interrupted(u64 *r, const u64 *a, const u64 *b, u64 *sum)
{
    u8 c = _addcarry_u64(0, a[0], b[0], (unsigned long long *)&r[0]);
    *sum += a[0] ^ b[0];
    c = _addcarry_u64(c, a[1], b[1], (unsigned long long *)&r[1]);
    *sum += (a[1] < b[1]) + 3;
    return _addcarry_u64(c, a[2], b[2], (unsigned long long *)&r[2]);
}

/* The result written over an operand. */
static NOINLINE u8 hdr_in_place(u64 *a, const u64 *b)
{
    u8 c = _addcarry_u64(0, a[0], b[0], (unsigned long long *)&a[0]);
    c = _addcarry_u64(c, a[1], b[1], (unsigned long long *)&a[1]);
    c = _addcarry_u64(c, a[2], b[2], (unsigned long long *)&a[2]);
    return _addcarry_u64(c, a[3], b[3], (unsigned long long *)&a[3]);
}

/* ---- 3. Loops. ---- */

static NOINLINE u8 hdr_addn(u64 *r, const u64 *a, const u64 *b, long n)
{
    u8 c = 0;
    for (long i = 0; i < n; i++)
        c = _addcarry_u64(c, a[i], b[i], (unsigned long long *)&r[i]);
    return c;
}

static NOINLINE u8 hdr_subn(u64 *r, const u64 *a, const u64 *b, long n)
{
    u8 c = 0;
    for (long i = 0; i < n; i++)
        c = _subborrow_u64(c, a[i], b[i], (unsigned long long *)&r[i]);
    return c;
}

/* Two links in each trip, and the carries counted. */
static NOINLINE unsigned hdr_addn_pairs(u64 *r, const u64 *a, const u64 *b, long n)
{
    u8 c = 0;
    unsigned count = 0;
    for (long i = 0; i + 1 < n; i += 2) {
        c = _addcarry_u64(c, a[i], b[i], (unsigned long long *)&r[i]);
        count += c;
        c = _addcarry_u64(c, a[i + 1], b[i + 1], (unsigned long long *)&r[i + 1]);
    }
    return count * 2 + c;
}

/* ---- 4. Across branches: the carry comes from one of two adders. ---- */

static NOINLINE u8 hdr_branchy(int which, u64 *r, const u64 *a, const u64 *b)
{
    u8 c;
    if (which & 1)
        c = _addcarry_u64(0, a[0], b[0], (unsigned long long *)&r[0]);
    else
        c = _subborrow_u64(0, a[0], b[0], (unsigned long long *)&r[0]);
    if (which & 2)
        c = _addcarry_u64(c, a[1], b[1], (unsigned long long *)&r[1]);
    else
        r[1] = a[1] ^ c;
    return _subborrow_u64(c, a[2], b[2], (unsigned long long *)&r[2]);
}
#endif /* HAVE_HEADER */

/* ---- 5. The builtins: their carry in is a whole word. ---- */

#if defined(__has_builtin)
#if __has_builtin(__builtin_addcll) && __has_builtin(__builtin_subcll)
#define HAVE_BUILTINS 1
#endif
#endif
#ifndef HAVE_BUILTINS
#define HAVE_BUILTINS 0
#endif

#if HAVE_BUILTINS
static NOINLINE u64 bi_addc(u64 a, u64 b, u64 cin, u64 *cout)
{
    unsigned long long o;
    u64 r = __builtin_addcll(a, b, cin, &o);
    *cout = o;
    return r;
}

static NOINLINE u64 bi_subc(u64 a, u64 b, u64 cin, u64 *cout)
{
    unsigned long long o;
    u64 r = __builtin_subcll(a, b, cin, &o);
    *cout = o;
    return r;
}

static NOINLINE u32 bi_addc32(u32 a, u32 b, u32 cin, u32 *cout)
{
    return __builtin_addc(a, b, cin, cout);
}

static NOINLINE u32 bi_subc32(u32 a, u32 b, u32 cin, u32 *cout)
{
    return __builtin_subc(a, b, cin, cout);
}

static NOINLINE u64 bi_add4(u64 *r, const u64 *a, const u64 *b)
{
    unsigned long long c;
    r[0] = __builtin_addcll(a[0], b[0], 0, &c);
    r[1] = __builtin_addcll(a[1], b[1], c, &c);
    r[2] = __builtin_addcll(a[2], b[2], c, &c);
    r[3] = __builtin_addcll(a[3], b[3], c, &c);
    return c;
}

static NOINLINE u64 bi_sub4(u64 *r, const u64 *a, const u64 *b)
{
    unsigned long long c;
    r[0] = __builtin_subcll(a[0], b[0], 0, &c);
    r[1] = __builtin_subcll(a[1], b[1], c, &c);
    r[2] = __builtin_subcll(a[2], b[2], c, &c);
    r[3] = __builtin_subcll(a[3], b[3], c, &c);
    return c;
}

static NOINLINE u32 bi_add4_32(u32 *r, const u32 *a, const u32 *b)
{
    unsigned c;
    r[0] = __builtin_addc(a[0], b[0], 0, &c);
    r[1] = __builtin_addc(a[1], b[1], c, &c);
    r[2] = __builtin_addc(a[2], b[2], c, &c);
    r[3] = __builtin_addc(a[3], b[3], c, &c);
    return c;
}
#endif /* HAVE_BUILTINS */

/* ---- 6. Comparisons. ---- */

/* The carry in is a bit because of where it comes from. */
static NOINLINE u64 cmp_add(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    u64 s = a + b;
    u64 c1 = s < a;
    u64 t = s + cin;
    u64 c2 = t < s;
    *cout = c1 | c2;
    return t;
}

/* The other operand in both comparisons. */
static NOINLINE u64 cmp_add_other(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    u64 s = a + b;
    u64 t = cin + s;
    *cout = (s < b) | (t < cin);
    return t;
}

static NOINLINE u64 cmp_sub(u64 a, u64 b, u64 x, u64 y, u64 *bout)
{
    u64 bin = x > y;
    u64 d = a - b;
    u64 e = d - bin;
    *bout = (a < b) | (d < bin);
    return e;
}

/* `e > d`: the difference grew. */
static NOINLINE u64 cmp_sub_grew(u64 a, u64 b, u64 x, u64 y, u64 *bout)
{
    u64 bin = x > y;
    u64 d = a - b;
    u64 e = d - bin;
    *bout = (b > a) | (e > d);
    return e;
}

static NOINLINE u32 cmp_add32(u32 a, u32 b, u32 x, u32 y, u32 *cout)
{
    u32 cin = x != y;
    u32 s = a + b;
    u32 t = s + cin;
    *cout = (s < a) | (t < s);
    return t;
}

static NOINLINE u32 cmp_sub32(u32 a, u32 b, u32 x, u32 y, u32 *bout)
{
    u32 bin = x == y;
    u32 d = a - b;
    u32 e = d - bin;
    *bout = (a < b) | (d < bin);
    return e;
}

/* A masked bit as the carry in. */
static NOINLINE u64 cmp_add_masked(u64 a, u64 b, u64 word, u64 *cout)
{
    u64 cin = word & 1;
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) | (t < s);
    return t;
}

/* A carry in that is a whole word: no `adc` computes this. */
static NOINLINE u64 cmp_add_word(u64 a, u64 b, u64 cin, u64 *cout)
{
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) | (t < s);
    return t;
}

static NOINLINE u64 cmp_sub_word(u64 a, u64 b, u64 bin, u64 *bout)
{
    u64 d = a - b;
    u64 e = d - bin;
    *bout = (a < b) | (d < bin);
    return e;
}

/* The carry in read from memory: a word as far as anybody can tell. */
static NOINLINE u64 cmp_add_loaded(u64 a, u64 b, const u64 *cin, u64 *cout)
{
    u64 s = a + b;
    u64 t = s + *cin;
    *cout = (s < a) | (t < s);
    return t;
}

/* Other ways to join the two carries, and near misses. */
static NOINLINE u64 cmp_add_logical(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) || (t < s);
    return t;
}

static NOINLINE u64 cmp_add_plus(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) + (t < s);
    return t;
}

static NOINLINE u64 cmp_add_xor(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) ^ (t < s);
    return t;
}

/* Not an adder: the second comparison is against the wrong sum. */
static NOINLINE u64 cmp_not_an_adder(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) | (t < a);
    return t;
}

/* Not an adder: signed comparisons. */
static NOINLINE u64 cmp_signed(int64_t a, int64_t b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    int64_t s = (int64_t)((u64)a + (u64)b);
    int64_t t = (int64_t)((u64)s + cin);
    *cout = (u64)(s < a) | (u64)(t < s);
    return (u64)t;
}

/* The partial carries are read on their own as well. */
static NOINLINE u64 cmp_add_shared(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cin = x < y;
    u64 s = a + b;
    u64 c1 = s < a;
    u64 t = s + cin;
    u64 c2 = t < s;
    *cout = (c1 | c2) + 4 * c1 + 16 * c2;
    return t;
}

/* A chain of limbs. */
static NOINLINE u64 cmp_add4(u64 *r, const u64 *a, const u64 *b)
{
    u64 c = 0;
    for (int i = 0; i < 4; i++) {
        u64 s = a[i] + b[i];
        u64 c1 = s < a[i];
        u64 t = s + c;
        c = c1 | (t < s);
        r[i] = t;
    }
    return c;
}

static NOINLINE u64 cmp_subn(u64 *r, const u64 *a, const u64 *b, long n)
{
    u64 borrow = 0;
    for (long i = 0; i < n; i++) {
        u64 d = a[i] - b[i];
        u64 b1 = a[i] < b[i];
        u64 e = d - borrow;
        borrow = b1 | (d < borrow);
        r[i] = e;
    }
    return borrow;
}

static NOINLINE u32 cmp_addn32(u32 *r, const u32 *a, const u32 *b, long n)
{
    u32 c = 0;
    for (long i = 0; i < n; i++) {
        u32 s = a[i] + b[i];
        u32 c1 = s < a[i];
        u32 t = s + c;
        c = c1 | (t < s);
        r[i] = t;
    }
    return c;
}

/* ---- 7. Other orders, other joins, other first borrows. ---- */

/* The carry added first, the two carries added up (mini-gmp's order). */
static NOINLINE u64 cmp_add_carry_first(u64 a, u64 b, u64 x, u64 y, u64 *cout)
{
    u64 cy = x < y;
    u64 r = a + cy;
    cy = r < cy;
    r += b;
    cy += r < b;
    *cout = cy;
    return r;
}

/* The borrow subtracted first. */
static NOINLINE u64 cmp_sub_borrow_first(u64 a, u64 b, u64 x, u64 y, u64 *bout)
{
    u64 bin = x > y;
    u64 d = a - bin;
    u64 e = d - b;
    *bout = (a < bin) + (d < b);
    return e;
}

/* Both borrows spelled "the difference is the greater" (GMP's sub_n). */
static NOINLINE u64 cmp_sub_both_grew(u64 a, u64 b, u64 x, u64 y, u64 *bout)
{
    u64 bin = x > y;
    u64 d = a - b;
    u64 b1 = d > a;
    u64 e = d - bin;
    u64 b2 = e > d;
    *bout = b1 | b2;
    return e;
}

/* The borrow computed ahead of the difference. */
static NOINLINE u64 cmp_sub_borrow_ahead(u64 a, u64 b, u64 x, u64 y, u64 *bout)
{
    u64 bin = x > y;
    u64 d = a - b;
    *bout = (a < b) | (d < bin);
    return d - bin;
}

/* The subtrahends added up first (mini-gmp's order). */
static NOINLINE u64 cmp_sub_of_sum(u64 a, u64 b, u64 x, u64 y, u64 *bout)
{
    u64 cy = x > y;
    b += cy;
    cy = b < cy;
    cy += a < b;
    *bout = cy;
    return a - b;
}

/* Not an adder: two words added up and subtracted borrow twice. */
static NOINLINE u64 cmp_sub_of_sum_word(u64 a, u64 b, u64 w, u64 *bout)
{
    u64 s = b + w;
    u64 n = s < w;
    n += a < s;
    *bout = n;
    return a - s;
}

/* `||` of the carries, the carry in an `int` that is not a bit. */
static NOINLINE int cmp_add_logical_int(u64 a, u64 b, int cin, u64 *r)
{
    u64 s = a + b;
    u64 t = s + (cin != 0);
    *r = t;
    return s < a || t < s;
}

/* Not adders: the carries of a sum of three words are counted, and there
 * may be two of them. */
static NOINLINE u64 cmp_add_plus_word(u64 a, u64 b, u64 cin, u64 *cout)
{
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) + (t < s);
    return t;
}

static NOINLINE u64 cmp_add_xor_word(u64 a, u64 b, u64 cin, u64 *cout)
{
    u64 s = a + b;
    u64 t = s + cin;
    *cout = (s < a) ^ (t < s);
    return t;
}

static NOINLINE u64 cmp_sub_plus_word(u64 a, u64 b, u64 bin, u64 *bout)
{
    u64 d = a - b;
    u64 e = d - bin;
    *bout = (a < b) + (d < bin);
    return e;
}

/* Not an adder either: the minuend is a bit, the subtrahends are words,
 * and `1 - y - w` can borrow twice. */
static NOINLINE u64 cmp_sub_from_bit(u64 p, u64 q, u64 y, u64 w, u64 *bout)
{
    u64 x = p < q;
    u64 d = x - y;
    u64 e = d - w;
    *bout = (x < y) + (d < w);
    return e;
}

/* ---- 8. Limb loops as libraries write them. ---- */

/* GMP, mpn/generic/add_n.c and sub_n.c (n >= 1). */
static NOINLINE u64 lib_gmp_add_n(u64 *rp, const u64 *up, const u64 *vp, long n)
{
    u64 ul, vl, sl, rl, cy, cy1, cy2;
    cy = 0;
    do {
        ul = *up++;
        vl = *vp++;
        sl = ul + vl;
        cy1 = sl < ul;
        rl = sl + cy;
        cy2 = rl < sl;
        cy = cy1 | cy2;
        *rp++ = rl;
    } while (--n != 0);
    return cy;
}

static NOINLINE u64 lib_gmp_sub_n(u64 *rp, const u64 *up, const u64 *vp, long n)
{
    u64 ul, vl, sl, rl, cy, cy1, cy2;
    cy = 0;
    do {
        ul = *up++;
        vl = *vp++;
        sl = ul - vl;
        cy1 = sl > ul;
        rl = sl - cy;
        cy2 = rl > sl;
        cy = cy1 | cy2;
        *rp++ = rl;
    } while (--n != 0);
    return cy;
}

/* mini-gmp.c, mpn_add_n and mpn_sub_n. */
static NOINLINE u64 lib_mini_add_n(u64 *rp, const u64 *ap, const u64 *bp, long n)
{
    long i;
    u64 cy;
    for (i = 0, cy = 0; i < n; i++) {
        u64 a, b, r;
        a = ap[i];
        b = bp[i];
        r = a + cy;
        cy = (r < cy);
        r += b;
        cy += (r < b);
        rp[i] = r;
    }
    return cy;
}

static NOINLINE u64 lib_mini_sub_n(u64 *rp, const u64 *ap, const u64 *bp, long n)
{
    long i;
    u64 cy;
    for (i = 0, cy = 0; i < n; i++) {
        u64 a, b;
        a = ap[i];
        b = bp[i];
        b += cy;
        cy = (b < cy);
        cy += (a < b);
        rp[i] = a - b;
    }
    return cy;
}

/* OpenSSL, crypto/bn/bn_asm.c, bn_add_words and bn_sub_words without
 * BN_LLONG, one limb a trip. */
static NOINLINE u64 lib_ossl_add_words(u64 *r, const u64 *a, const u64 *b, int n)
{
    u64 c, l, t;
    if (n <= 0)
        return 0;
    c = 0;
    while (n) {
        t = a[0];
        t = (t + c) & 0xffffffffffffffffull;
        c = (t < c);
        l = (t + b[0]) & 0xffffffffffffffffull;
        c += (l < t);
        r[0] = l;
        a++;
        b++;
        r++;
        n--;
    }
    return c;
}

static NOINLINE u64 lib_ossl_sub_words(u64 *r, const u64 *a, const u64 *b, int n)
{
    u64 t1, t2;
    int c = 0;
    if (n <= 0)
        return 0;
    while (n) {
        t1 = a[0];
        t2 = b[0];
        r[0] = (t1 - t2 - c) & 0xffffffffffffffffull;
        if (t1 != t2)
            c = (t1 < t2);
        a++;
        b++;
        r++;
        n--;
    }
    return (u64)c;
}

/* The double word. */
#if defined(__SIZEOF_INT128__)
#define HAVE_INT128 1
static NOINLINE u64 lib_i128_add_n(u64 *r, const u64 *a, const u64 *b, long n)
{
    u64 c = 0;
    for (long i = 0; i < n; i++) {
        unsigned __int128 t = (unsigned __int128)a[i] + b[i] + c;
        r[i] = (u64)t;
        c = (u64)(t >> 64);
    }
    return c;
}

static NOINLINE u64 lib_i128_sub_n(u64 *r, const u64 *a, const u64 *b, long n)
{
    u64 borrow = 0;
    for (long i = 0; i < n; i++) {
        unsigned __int128 t = (unsigned __int128)a[i] - b[i] - borrow;
        r[i] = (u64)t;
        borrow = (u64)(t >> 64) & 1;
    }
    return borrow;
}

/* Three words in the double word: the high half counts to two. */
static NOINLINE u64 lib_i128_add3_n(u64 *r, const u64 *a, const u64 *b, const u64 *w, long n)
{
    u64 c = 0;
    for (long i = 0; i < n; i++) {
        unsigned __int128 t = (unsigned __int128)a[i] + b[i] + w[i];
        r[i] = (u64)t ^ c;
        c = (u64)(t >> 64);
    }
    return c;
}
#else
#define HAVE_INT128 0
#endif

/* The overflow builtins. */
static NOINLINE u64 lib_ovf_add_n(u64 *r, const u64 *a, const u64 *b, long n)
{
    u64 c = 0;
    for (long i = 0; i < n; i++) {
        unsigned long long s, t;
        u64 c1 = __builtin_add_overflow((unsigned long long)a[i], (unsigned long long)b[i], &s);
        u64 c2 = __builtin_add_overflow(s, (unsigned long long)c, &t);
        r[i] = t;
        c = c1 | c2;
    }
    return c;
}

static NOINLINE u64 lib_ovf_sub_n(u64 *r, const u64 *a, const u64 *b, long n)
{
    u64 c = 0;
    for (long i = 0; i < n; i++) {
        unsigned long long s, t;
        u64 c1 = __builtin_sub_overflow((unsigned long long)a[i], (unsigned long long)b[i], &s);
        u64 c2 = __builtin_sub_overflow(s, (unsigned long long)c, &t);
        r[i] = t;
        c = c1 + c2;
    }
    return c;
}

/* The carry a `_Bool`, the carries joined by `||`. */
static NOINLINE u64 lib_bool_add_n(u64 *r, const u64 *a, const u64 *b, long n)
{
    _Bool c = 0;
    for (long i = 0; i < n; i++) {
        u64 s = a[i] + b[i];
        u64 t = s + c;
        c = s < a[i] || t < s;
        r[i] = t;
    }
    return c;
}

static NOINLINE u64 lib_bool_sub_n(u64 *r, const u64 *a, const u64 *b, long n)
{
    _Bool borrow = 0;
    for (long i = 0; i < n; i++) {
        u64 d = a[i] - b[i];
        u64 e = d - borrow;
        borrow = a[i] < b[i] || d < (u64)borrow;
        r[i] = e;
    }
    return borrow;
}

/* Not adders: the carries of three words a trip are counted (0, 1 or 2),
 * and the count is what goes round. */
static NOINLINE u64 lib_count_n(u64 *r, const u64 *a, const u64 *b, const u64 *w, long n)
{
    u64 c = 0;
    for (long i = 0; i < n; i++) {
        u64 s = a[i] + b[i];
        u64 t = s + w[i];
        r[i] = t ^ c;
        c = (s < a[i]) + (t < s);
    }
    return c;
}

/* The count is the third word of the next trip: it starts as a word. */
static NOINLINE u64 lib_count_carried_n(u64 *r, const u64 *a, const u64 *b, u64 c, long n)
{
    for (long i = 0; i < n; i++) {
        u64 s = a[i] + b[i];
        u64 t = s + c;
        r[i] = t;
        c = (s < a[i]) + (t < s);
    }
    return c;
}

/* ---- The driver. ---- */

#define LIMBS 9

static void fill(u64 *a, u64 *b, int n)
{
    for (int i = 0; i < n; i++) {
        a[i] = limb();
        b[i] = limb();
    }
}

/* r = a + b + c over n limbs; returns the carry out. */
static unsigned ref_addn(u64 *r, const u64 *a, const u64 *b, int n, unsigned c)
{
    for (int i = 0; i < n; i++)
        r[i] = ref_add64(a[i], b[i], c, &c);
    return c;
}

static unsigned ref_subn(u64 *r, const u64 *a, const u64 *b, int n, unsigned c)
{
    for (int i = 0; i < n; i++)
        r[i] = ref_sub64(a[i], b[i], c, &c);
    return c;
}

static void expect_limbs(const char *what, const u64 *got, const u64 *want, int n)
{
    for (int i = 0; i < n; i++)
        expect(what, got[i], want[i]);
}

static void single_adders(u64 a, u64 b)
{
    unsigned want_c;
    u64 want;
    (void)a;
    (void)b;
    (void)want;
    (void)want_c;
#if HAVE_HEADER
    for (unsigned k = 0; k < NCARRIES; k++) {
        u8 c = carries[k];
        u64 r;
        u32 r32;
        u8 o;

        want = ref_add64(a, b, c, &want_c);
        o = hdr_add64(c, a, b, &r);
        expect("hdr_add64", r, want);
        expect("hdr_add64 carry", o, want_c);
        expect("hdr_carry_only", hdr_carry_only(c, a, b), want_c);

        want = ref_sub64(a, b, c, &want_c);
        o = hdr_sub64(c, a, b, &r);
        expect("hdr_sub64", r, want);
        expect("hdr_sub64 borrow", o, want_c);
        expect("hdr_word_only", hdr_word_only(c, a, b), want);

        want = ref_add32((u32)a, (u32)b, c, &want_c);
        o = hdr_add32(c, (u32)a, (u32)b, &r32);
        expect("hdr_add32", r32, want);
        expect("hdr_add32 carry", o, want_c);
        expect("hdr_add32_wide", hdr_add32_wide(c, (u32)a, (u32)b),
               ((u64)want_c << 40) ^ want ^ (want << 3));

        want = ref_sub32((u32)a, (u32)b, c, &want_c);
        o = hdr_sub32(c, (u32)a, (u32)b, &r32);
        expect("hdr_sub32", r32, want);
        expect("hdr_sub32 borrow", o, want_c);

        want = ref_add64(a, 1, c, &want_c);
        o = hdr_add_one(c, a, &r);
        expect("hdr_add_one", r, want);
        expect("hdr_add_one carry", o, want_c);

        want = ref_add64(a, ~0ull, c, &want_c);
        o = hdr_add_minus_one(c, a, &r);
        expect("hdr_add_minus_one", r, want);
        expect("hdr_add_minus_one carry", o, want_c);

        want = ref_sub64(a, 0x123456789ull, c, &want_c);
        o = hdr_sub_big(c, a, &r);
        expect("hdr_sub_big", r, want);
        expect("hdr_sub_big borrow", o, want_c);

        want = ref_add32((u32)a, 0xffffffffu, c, &want_c);
        o = hdr_add32_all_ones(c, (u32)a, &r32);
        expect("hdr_add32_all_ones", r32, want);
        expect("hdr_add32_all_ones carry", o, want_c);

        want = ref_add64(a, a, c, &want_c);
        o = hdr_double(c, a, &r);
        expect("hdr_double", r, want);
        expect("hdr_double carry", o, want_c);

        want = ref_sub64(0, a, c, &want_c);
        o = hdr_negate(c, a, &r);
        expect("hdr_negate", r, want);
        expect("hdr_negate borrow", o, want_c);

        want = ref_add64(0xfedcba9876543210ull, b, c, &want_c);
        o = hdr_from_constant(c, b, &r);
        expect("hdr_from_constant", r, want);
        expect("hdr_from_constant carry", o, want_c);
    }
    {
        u64 r;
        u8 o;
        want = ref_add64(a, b, 1, &want_c);
        o = hdr_add_carry_set(a, b, &r);
        expect("hdr_add_carry_set", r, want);
        expect("hdr_add_carry_set carry", o, want_c);
        want = ref_sub64(a, b, 1, &want_c);
        o = hdr_sub_borrow_set(a, b, &r);
        expect("hdr_sub_borrow_set", r, want);
        expect("hdr_sub_borrow_set borrow", o, want_c);
    }
#endif
}

/* The builtins and the comparisons with a carry in that is a whole word:
 * the sum of three words, and whether either addition wrapped. */
static u64 ref_add_word(u64 a, u64 b, u64 cin, u64 *cout)
{
    unsigned c1, c2;
    u64 s = ref_add64(a, b, 0, &c1);
    u64 t = ref_add64(s, cin, 0, &c2);
    *cout = c1 | c2;
    return t;
}

static u64 ref_sub_word(u64 a, u64 b, u64 bin, u64 *bout)
{
    unsigned b1, b2;
    u64 d = ref_sub64(a, b, 0, &b1);
    u64 e = ref_sub64(d, bin, 0, &b2);
    *bout = b1 | b2;
    return e;
}

static void word_carries(u64 a, u64 b, u64 cin)
{
    u64 want, want_c, r, o;

    want = ref_add_word(a, b, cin, &want_c);
    r = cmp_add_word(a, b, cin, &o);
    expect("cmp_add_word", r, want);
    expect("cmp_add_word carry", o, want_c);
    r = cmp_add_loaded(a, b, &cin, &o);
    expect("cmp_add_loaded", r, want);
    expect("cmp_add_loaded carry", o, want_c);
    r = cmp_add_masked(a, b, cin, &o);
    {
        unsigned c;
        expect("cmp_add_masked", r, ref_add64(a, b, cin & 1, &c));
        expect("cmp_add_masked carry", o, c);
    }

    want = ref_sub_word(a, b, cin, &want_c);
    r = cmp_sub_word(a, b, cin, &o);
    expect("cmp_sub_word", r, want);
    expect("cmp_sub_word borrow", o, want_c);
    {
        unsigned c;
        int word = (int)(cin & 0x7fffffffu);
        o = (u64)cmp_add_logical_int(a, b, word, &r);
        expect("cmp_add_logical_int", r, ref_add64(a, b, word != 0, &c));
        expect("cmp_add_logical_int carry", o, c);
    }
    {
        /* The two carries on their own: counted, and exclusive-or'ed. */
        unsigned c1, c2;
        u64 s = ref_add64(a, b, 0, &c1);
        u64 t = ref_add64(s, cin, 0, &c2);
        r = cmp_add_plus_word(a, b, cin, &o);
        expect("cmp_add_plus_word", r, t);
        expect("cmp_add_plus_word carries", o, c1 + c2);
        r = cmp_add_xor_word(a, b, cin, &o);
        expect("cmp_add_xor_word", r, t);
        expect("cmp_add_xor_word carries", o, c1 ^ c2);
        s = ref_sub64(a, b, 0, &c1);
        t = ref_sub64(s, cin, 0, &c2);
        r = cmp_sub_plus_word(a, b, cin, &o);
        expect("cmp_sub_plus_word", r, t);
        expect("cmp_sub_plus_word borrows", o, c1 + c2);
        /* A minuend of 0 or 1 and two words. */
        s = ref_sub64(a < b, b, 0, &c1);
        t = ref_sub64(s, cin, 0, &c2);
        r = cmp_sub_from_bit(a, b, b, cin, &o);
        expect("cmp_sub_from_bit", r, t);
        expect("cmp_sub_from_bit borrows", o, c1 + c2);
        /* Two words added up, and their sum subtracted. */
        s = ref_add64(b, cin, 0, &c1);
        t = ref_sub64(a, s, 0, &c2);
        r = cmp_sub_of_sum_word(a, b, cin, &o);
        expect("cmp_sub_of_sum_word", r, t);
        expect("cmp_sub_of_sum_word borrows", o, c1 + c2);
    }

#if HAVE_BUILTINS
    want = ref_add_word(a, b, cin, &want_c);
    r = bi_addc(a, b, cin, &o);
    expect("bi_addc", r, want);
    expect("bi_addc carry", o, want_c);
    want = ref_sub_word(a, b, cin, &want_c);
    r = bi_subc(a, b, cin, &o);
    expect("bi_subc", r, want);
    expect("bi_subc borrow", o, want_c);
    {
        u32 o32, r32;
        u64 s = (u64)(u32)a + (u32)b;
        u64 t = (s & 0xffffffffu) + (u32)cin;
        r32 = bi_addc32((u32)a, (u32)b, (u32)cin, &o32);
        expect("bi_addc32", r32, (u32)t);
        expect("bi_addc32 carry", o32, (s >> 32) | (t >> 32));
        s = (u64)(u32)a - (u32)b;
        t = (s & 0xffffffffu) - (u32)cin;
        r32 = bi_subc32((u32)a, (u32)b, (u32)cin, &o32);
        expect("bi_subc32", r32, (u32)t);
        expect("bi_subc32 borrow", o32, (s >> 63) | (t >> 63));
    }
#endif
}

static void compared(u64 a, u64 b, u64 x, u64 y)
{
    unsigned want_c;
    u64 want, r, o;
    u32 r32, o32;

    want = ref_add64(a, b, x < y, &want_c);
    r = cmp_add(a, b, x, y, &o);
    expect("cmp_add", r, want);
    expect("cmp_add carry", o, want_c);
    r = cmp_add_other(a, b, x, y, &o);
    expect("cmp_add_other", r, want);
    expect("cmp_add_other carry", o, want_c);
    r = cmp_add_logical(a, b, x, y, &o);
    expect("cmp_add_logical", r, want);
    expect("cmp_add_logical carry", o, want_c);
    r = cmp_add_plus(a, b, x, y, &o);
    expect("cmp_add_plus", r, want);
    expect("cmp_add_plus carry", o, want_c);
    r = cmp_add_xor(a, b, x, y, &o);
    expect("cmp_add_xor", r, want);
    expect("cmp_add_xor carry", o, want_c);
    r = cmp_add_carry_first(a, b, x, y, &o);
    expect("cmp_add_carry_first", r, want);
    expect("cmp_add_carry_first carry", o, want_c);
    r = cmp_add_shared(a, b, x, y, &o);
    expect("cmp_add_shared", r, want);
    {
        unsigned c1, c2;
        u64 s = ref_add64(a, b, 0, &c1);
        (void)ref_add64(s, x < y, 0, &c2);
        expect("cmp_add_shared carries", o, (c1 | c2) + 4 * c1 + 16 * c2);
        /* The near miss: `t < a` instead of `t < s`. */
        r = cmp_not_an_adder(a, b, x, y, &o);
        expect("cmp_not_an_adder", r, want);
        expect("cmp_not_an_adder bits", o, c1 | (want < a));
    }
    r = cmp_signed((int64_t)a, (int64_t)b, x, y, &o);
    expect("cmp_signed", r, want);
    {
        int64_t s = (int64_t)(a + b);
        expect("cmp_signed bits", o,
               (u64)(s < (int64_t)a) | (u64)((int64_t)want < s));
    }

    want = ref_sub64(a, b, x > y, &want_c);
    r = cmp_sub(a, b, x, y, &o);
    expect("cmp_sub", r, want);
    expect("cmp_sub borrow", o, want_c);
    r = cmp_sub_grew(a, b, x, y, &o);
    expect("cmp_sub_grew", r, want);
    expect("cmp_sub_grew borrow", o, want_c);
    r = cmp_sub_borrow_first(a, b, x, y, &o);
    expect("cmp_sub_borrow_first", r, want);
    expect("cmp_sub_borrow_first borrow", o, want_c);
    r = cmp_sub_both_grew(a, b, x, y, &o);
    expect("cmp_sub_both_grew", r, want);
    expect("cmp_sub_both_grew borrow", o, want_c);
    r = cmp_sub_borrow_ahead(a, b, x, y, &o);
    expect("cmp_sub_borrow_ahead", r, want);
    expect("cmp_sub_borrow_ahead borrow", o, want_c);
    r = cmp_sub_of_sum(a, b, x, y, &o);
    expect("cmp_sub_of_sum", r, want);
    expect("cmp_sub_of_sum borrow", o, want_c);

    want = ref_add32((u32)a, (u32)b, (u32)x != (u32)y, &want_c);
    r32 = cmp_add32((u32)a, (u32)b, (u32)x, (u32)y, &o32);
    expect("cmp_add32", r32, want);
    expect("cmp_add32 carry", o32, want_c);
    want = ref_sub32((u32)a, (u32)b, (u32)x == (u32)y, &want_c);
    r32 = cmp_sub32((u32)a, (u32)b, (u32)x, (u32)y, &o32);
    expect("cmp_sub32", r32, want);
    expect("cmp_sub32 borrow", o32, want_c);
}

/* ---- 9. Two words: the carry is added, not chained (`hi += lo < x`). ---- */

/* GMP's longlong.h, generic C. */
static NOINLINE void two_add_ssaaaa(u64 *sh, u64 *sl, u64 ah, u64 al, u64 bh, u64 bl)
{
    u64 x = al + bl;
    *sh = ah + bh + (x < al);
    *sl = x;
}

static NOINLINE void two_sub_ddmmss(u64 *sh, u64 *sl, u64 ah, u64 al, u64 bh, u64 bl)
{
    u64 x = al - bl;
    *sh = ah - bh - (al < bl);
    *sl = x;
}

/* A 128-bit accumulator in memory. */
static NOINLINE void two_acc_add(u64 *hi, u64 *lo, u64 x)
{
    u64 l = *lo + x;
    *hi += l < x;
    *lo = l;
}

static NOINLINE void two_acc_sub(u64 *hi, u64 *lo, u64 x)
{
    u64 l = *lo;
    *hi -= l < x;
    *lo = l - x;
}

/* The column sum of a schoolbook product: each sum is stored before its
 * carry is read. */
static NOINLINE void two_acc3(u64 c[3], u64 lo, u64 hi)
{
    c[0] += lo;
    hi += c[0] < lo;
    c[1] += hi;
    c[2] += c[1] < hi;
}

/* The store between a sum and the reading of its carry may be to the word
 * that was added. The comparison then reads the sum itself, and is no
 * carry: the result of the addition must not stand in for it. */
static NOINLINE u64 two_stored_over_addend(u64 *r, const u64 *x, u64 hi)
{
    u64 s = r[0] + x[0];
    r[1] = s;
    hi += s < x[0];
    return hi;
}

static NOINLINE u64 two_stored_over_minuend(u64 *r, const u64 *x, u64 hi)
{
    u64 d = x[0] - r[0];
    r[1] = d;
    hi -= x[0] < r[0];
    return hi;
}

/* The same in a chain whose result goes over an operand. */
static NOINLINE unsigned two_sum_in_place(u64 *out, const u64 *a, u64 k)
{
    u64 s = a[0] + k;
    out[0] = s;
    return s < a[0];
}

static void double_words(const u64 *a, const u64 *b)
{
    for (int i = 0; i + 1 < LIMBS; i++) {
        unsigned c;
        u64 hi, lo, want_lo, want_hi, acc[3], want[3], top = a[(i + 2) % LIMBS];
        u64 pair[2], apart[2];

        /* Apart: an addition and its carry. Over the addend: the stored
         * sum is what is compared, and `s < s` is no carry. */
        pair[0] = a[i];
        pair[1] = 0;
        apart[0] = b[i];
        apart[1] = 0;
        hi = two_stored_over_addend(pair, apart, top);
        want_lo = ref_add64(a[i], b[i], 0, &c);
        expect("two_stored_over_addend sum", pair[1], want_lo);
        expect("two_stored_over_addend apart", hi, top + c);
        pair[0] = a[i];
        pair[1] = b[i];
        hi = two_stored_over_addend(pair, pair + 1, top);
        expect("two_stored_over_addend sum over the addend", pair[1], want_lo);
        expect("two_stored_over_addend over the addend", hi, top);

        /* A difference stored over its minuend: `d < r[0]` is whatever it
         * is, not the borrow of `x[0] - r[0]`. */
        pair[0] = a[i];
        pair[1] = 0;
        apart[0] = b[i];
        hi = two_stored_over_minuend(pair, apart, top);
        want_lo = ref_sub64(b[i], a[i], 0, &c);
        expect("two_stored_over_minuend difference", pair[1], want_lo);
        expect("two_stored_over_minuend apart", hi, top - c);
        pair[0] = a[i];
        pair[1] = b[i];
        hi = two_stored_over_minuend(pair, pair + 1, top);
        expect("two_stored_over_minuend difference over the minuend", pair[1], want_lo);
        expect("two_stored_over_minuend over the minuend", hi, top - (want_lo < a[i]));

        pair[0] = a[i];
        want_lo = ref_add64(a[i], b[i], 0, &c);
        expect("two_sum_in_place apart", two_sum_in_place(apart, pair, b[i]), c);
        expect("two_sum_in_place sum", apart[0], want_lo);
        expect("two_sum_in_place over the operand", two_sum_in_place(pair, pair, b[i]), 0);
        expect("two_sum_in_place sum over the operand", pair[0], want_lo);

        two_add_ssaaaa(&hi, &lo, a[i + 1], a[i], b[i + 1], b[i]);
        want_lo = ref_add64(a[i], b[i], 0, &c);
        want_hi = ref_add64(a[i + 1], b[i + 1], c, &c);
        expect("two_add_ssaaaa low", lo, want_lo);
        expect("two_add_ssaaaa high", hi, want_hi);

        two_sub_ddmmss(&hi, &lo, a[i + 1], a[i], b[i + 1], b[i]);
        want_lo = ref_sub64(a[i], b[i], 0, &c);
        want_hi = ref_sub64(a[i + 1], b[i + 1], c, &c);
        expect("two_sub_ddmmss low", lo, want_lo);
        expect("two_sub_ddmmss high", hi, want_hi);

        hi = a[i + 1];
        lo = a[i];
        two_acc_add(&hi, &lo, b[i]);
        want_lo = ref_add64(a[i], b[i], 0, &c);
        want_hi = ref_add64(a[i + 1], 0, c, &c);
        expect("two_acc_add low", lo, want_lo);
        expect("two_acc_add high", hi, want_hi);

        hi = a[i + 1];
        lo = a[i];
        two_acc_sub(&hi, &lo, b[i]);
        want_lo = ref_sub64(a[i], b[i], 0, &c);
        want_hi = ref_sub64(a[i + 1], 0, c, &c);
        expect("two_acc_sub low", lo, want_lo);
        expect("two_acc_sub high", hi, want_hi);

        acc[0] = a[i];
        acc[1] = a[i + 1];
        acc[2] = b[i + 1];
        two_acc3(acc, b[i], top);
        want[0] = ref_add64(a[i], b[i], 0, &c);
        top = ref_add64(top, 0, c, &c);
        want[1] = ref_add64(a[i + 1], top, 0, &c);
        want[2] = ref_add64(b[i + 1], 0, c, &c);
        expect_limbs("two_acc3", acc, want, 3);
    }
}

/* The loops of section 8, at every length. */
static void libraries(const u64 *a, const u64 *b)
{
    u64 r[LIMBS], want[LIMBS], w[LIMBS];
    unsigned want_c;
    for (int i = 0; i < LIMBS; i++)
        w[i] = limb();
    for (int n = 0; n <= LIMBS; n++) {
        for (int i = 0; i < LIMBS; i++)
            r[i] = want[i] = 0x3c3c3c3c3c3c3c3cull - (u64)i;

        want_c = ref_addn(want, a, b, n, 0);
        if (n > 0) {
            expect("lib_gmp_add_n carry", lib_gmp_add_n(r, a, b, n), want_c);
            expect_limbs("lib_gmp_add_n", r, want, LIMBS);
        }
        expect("lib_mini_add_n carry", lib_mini_add_n(r, a, b, n), want_c);
        expect_limbs("lib_mini_add_n", r, want, LIMBS);
        expect("lib_ossl_add_words carry", lib_ossl_add_words(r, a, b, n), want_c);
        expect_limbs("lib_ossl_add_words", r, want, LIMBS);
        expect("lib_ovf_add_n carry", lib_ovf_add_n(r, a, b, n), want_c);
        expect_limbs("lib_ovf_add_n", r, want, LIMBS);
        expect("lib_bool_add_n carry", lib_bool_add_n(r, a, b, n), want_c);
        expect_limbs("lib_bool_add_n", r, want, LIMBS);
#if HAVE_INT128
        expect("lib_i128_add_n carry", lib_i128_add_n(r, a, b, n), want_c);
        expect_limbs("lib_i128_add_n", r, want, LIMBS);
#endif

        want_c = ref_subn(want, a, b, n, 0);
        if (n > 0) {
            expect("lib_gmp_sub_n borrow", lib_gmp_sub_n(r, a, b, n), want_c);
            expect_limbs("lib_gmp_sub_n", r, want, LIMBS);
        }
        expect("lib_mini_sub_n borrow", lib_mini_sub_n(r, a, b, n), want_c);
        expect_limbs("lib_mini_sub_n", r, want, LIMBS);
        expect("lib_ossl_sub_words borrow", lib_ossl_sub_words(r, a, b, n), want_c);
        expect_limbs("lib_ossl_sub_words", r, want, LIMBS);
        expect("lib_ovf_sub_n borrow", lib_ovf_sub_n(r, a, b, n), want_c);
        expect_limbs("lib_ovf_sub_n", r, want, LIMBS);
        expect("lib_bool_sub_n borrow", lib_bool_sub_n(r, a, b, n), want_c);
        expect_limbs("lib_bool_sub_n", r, want, LIMBS);
#if HAVE_INT128
        expect("lib_i128_sub_n borrow", lib_i128_sub_n(r, a, b, n), want_c);
        expect_limbs("lib_i128_sub_n", r, want, LIMBS);
#endif

        {
            /* Three words a trip: the count of carries goes round. */
            u64 count = 0;
            for (int i = 0; i < n; i++) {
                unsigned c1, c2;
                u64 s = ref_add64(a[i], b[i], 0, &c1);
                u64 t = ref_add64(s, w[i], 0, &c2);
                want[i] = t ^ count;
                count = c1 + c2;
            }
            expect("lib_count_n count", lib_count_n(r, a, b, w, n), count);
            expect_limbs("lib_count_n", r, want, LIMBS);
#if HAVE_INT128
            expect("lib_i128_add3_n count", lib_i128_add3_n(r, a, b, w, n), count);
            expect_limbs("lib_i128_add3_n", r, want, LIMBS);
#endif
        }
        for (unsigned k = 0; k < NEDGES; k++) {
            /* The count is the third word, and the first one any word. */
            u64 count = edges[k];
            for (int i = 0; i < n; i++) {
                unsigned c1, c2;
                u64 s = ref_add64(a[i], b[i], 0, &c1);
                want[i] = ref_add64(s, count, 0, &c2);
                count = c1 + c2;
            }
            expect("lib_count_carried_n count", lib_count_carried_n(r, a, b, edges[k], n), count);
            expect_limbs("lib_count_carried_n", r, want, LIMBS);
        }
    }
}

static void chains(void)
{
    u64 a[LIMBS], b[LIMBS], r[LIMBS], want[LIMBS];
    unsigned want_c;
    fill(a, b, LIMBS);
    for (int i = 0; i < LIMBS; i++)
        r[i] = want[i] = 0x5a5a5a5a5a5a5a5aull + (u64)i;

    want_c = ref_addn(want, a, b, 4, 0);
    expect("cmp_add4 carry", cmp_add4(r, a, b), want_c);
    expect_limbs("cmp_add4", r, want, LIMBS);

    for (int n = 0; n <= LIMBS; n++) {
        want_c = ref_subn(want, a, b, n, 0);
        expect("cmp_subn borrow", cmp_subn(r, a, b, n), want_c);
        expect_limbs("cmp_subn", r, want, LIMBS);
    }
    libraries(a, b);
    double_words(a, b);
    {
        u32 a32[LIMBS], b32[LIMBS], r32[LIMBS], want32[LIMBS];
        for (int n = 0; n <= LIMBS; n++) {
            unsigned c = 0;
            for (int i = 0; i < LIMBS; i++) {
                a32[i] = (u32)a[i];
                b32[i] = (u32)(b[i] >> 7);
                r32[i] = want32[i] = 0xa5a5a5a5u;
            }
            for (int i = 0; i < n; i++)
                want32[i] = ref_add32(a32[i], b32[i], c, &c);
            expect("cmp_addn32 carry", cmp_addn32(r32, a32, b32, n), c);
            for (int i = 0; i < LIMBS; i++)
                expect("cmp_addn32", r32[i], want32[i]);
#if HAVE_HEADER
            if (n == 4) {
                expect("hdr_add4_32 carry", hdr_add4_32(r32, a32, b32), c);
                for (int i = 0; i < LIMBS; i++)
                    expect("hdr_add4_32", r32[i], want32[i]);
            }
#endif
#if HAVE_BUILTINS
            if (n == 4) {
                expect("bi_add4_32 carry", bi_add4_32(r32, a32, b32), c);
                for (int i = 0; i < LIMBS; i++)
                    expect("bi_add4_32", r32[i], want32[i]);
            }
#endif
        }
    }

#if HAVE_BUILTINS
    want_c = ref_addn(want, a, b, 4, 0);
    expect("bi_add4 carry", bi_add4(r, a, b), want_c);
    expect_limbs("bi_add4", r, want, LIMBS);
    want_c = ref_subn(want, a, b, 4, 0);
    expect("bi_sub4 borrow", bi_sub4(r, a, b), want_c);
    expect_limbs("bi_sub4", r, want, LIMBS);
#endif

#if HAVE_HEADER
    (void)ref_addn(want, a, b, 4, 0);
    hdr_add4(r, a, b);
    expect_limbs("hdr_add4", r, want, LIMBS);

    want_c = ref_subn(want, a, b, 4, 0);
    expect("hdr_sub4 borrow", hdr_sub4(r, a, b), want_c);
    expect_limbs("hdr_sub4", r, want, LIMBS);

    for (unsigned k = 0; k < NCARRIES; k++) {
        want_c = ref_addn(want, a, b, 3, carries[k]);
        expect("hdr_add3_carry_in carry", hdr_add3_carry_in(carries[k], r, a, b), want_c);
        expect_limbs("hdr_add3_carry_in", r, want, LIMBS);
    }
    {
        unsigned c = 0, n = 0;
        for (int i = 0; i < 4; i++) {
            want[i] = ref_add64(a[i], b[i], c, &c);
            n += c;
        }
        expect("hdr_add4_counted count", hdr_add4_counted(r, a, b), n);
        expect_limbs("hdr_add4_counted", r, want, LIMBS);

        c = 0;
        want[0] = ref_add64(a[0], b[0], c, &c);
        want[1] = ref_sub64(a[1], b[1], c, &c);
        want[2] = ref_add64(a[2], b[2], c, &c);
        want[3] = ref_sub64(a[3], b[3], c, &c);
        expect("hdr_mixed borrow", hdr_mixed(r, a, b), c);
        expect_limbs("hdr_mixed", r, want, LIMBS);
    }
    {
        u64 sum = 17, want_sum = 17;
        want_c = ref_addn(want, a, b, 3, 0);
        want_sum += a[0] ^ b[0];
        want_sum += (a[1] < b[1]) + 3;
        expect("hdr_interrupted carry", hdr_interrupted(r, a, b, &sum), want_c);
        expect("hdr_interrupted sum", sum, want_sum);
        expect_limbs("hdr_interrupted", r, want, LIMBS);
    }
    {
        u64 copy[LIMBS];
        for (int i = 0; i < LIMBS; i++)
            copy[i] = want[i] = a[i];
        want_c = ref_addn(want, a, b, 4, 0);
        expect("hdr_in_place carry", hdr_in_place(copy, b), want_c);
        expect_limbs("hdr_in_place", copy, want, LIMBS);
    }
    for (int n = 0; n <= LIMBS; n++) {
        for (int i = 0; i < LIMBS; i++)
            r[i] = want[i] = 0x1111111111111111ull * (u64)(i + 1);
        want_c = ref_addn(want, a, b, n, 0);
        expect("hdr_addn carry", hdr_addn(r, a, b, n), want_c);
        expect_limbs("hdr_addn", r, want, LIMBS);

        want_c = ref_subn(want, a, b, n, 0);
        expect("hdr_subn borrow", hdr_subn(r, a, b, n), want_c);
        expect_limbs("hdr_subn", r, want, LIMBS);

        {
            unsigned c = 0, count = 0;
            for (int i = 0; i + 1 < n; i += 2) {
                want[i] = ref_add64(a[i], b[i], c, &c);
                count += c;
                want[i + 1] = ref_add64(a[i + 1], b[i + 1], c, &c);
            }
            expect("hdr_addn_pairs count", hdr_addn_pairs(r, a, b, n), count * 2 + c);
            expect_limbs("hdr_addn_pairs", r, want, LIMBS);
        }
    }
    for (int which = 0; which < 4; which++) {
        unsigned c;
        for (int i = 0; i < LIMBS; i++)
            r[i] = want[i] = 0x7777777777777777ull;
        if (which & 1)
            want[0] = ref_add64(a[0], b[0], 0, &c);
        else
            want[0] = ref_sub64(a[0], b[0], 0, &c);
        if (which & 2)
            want[1] = ref_add64(a[1], b[1], c, &c);
        else
            want[1] = a[1] ^ c;
        want[2] = ref_sub64(a[2], b[2], c, &c);
        expect("hdr_branchy borrow", hdr_branchy(which, r, a, b), c);
        expect_limbs("hdr_branchy", r, want, LIMBS);
    }
#endif
}

int main(void)
{
    for (unsigned i = 0; i < NEDGES; i++)
        for (unsigned j = 0; j < NEDGES; j++) {
            single_adders(edges[i], edges[j]);
            for (unsigned k = 0; k < NEDGES; k++)
                word_carries(edges[i], edges[j], edges[k]);
            compared(edges[i], edges[j], edges[(i + j) % NEDGES], edges[(i * j + 1) % NEDGES]);
            compared(edges[i], edges[j], 1, 2);
            compared(edges[i], edges[j], 2, 1);
        }
    printf("edges    %016llx\n", (unsigned long long)checksum);
    for (int round = 0; round < 3000; round++) {
        u64 a = limb(), b = limb(), x = limb(), y = limb();
        single_adders(a, b);
        word_carries(a, b, x);
        compared(a, b, x, y);
    }
    printf("random   %016llx\n", (unsigned long long)checksum);
    for (int round = 0; round < 2000; round++)
        chains();
    printf("chains   %016llx\n", (unsigned long long)checksum);
    printf("failures %lu\n", failures);
    return failures != 0;
}
