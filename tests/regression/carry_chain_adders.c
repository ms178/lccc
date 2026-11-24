/* Full adders: `x + y + carry` and `x - y - borrow` with their carry out,
 * in every spelling C has for them -- the header intrinsics
 * (`_addcarry_u64`, `_subborrow_u32`, ...), the builtins
 * (`__builtin_addcll`, `__builtin_subc`, ...) and comparisons
 * (`s = a + b; c = s < a; t = s + cin; c |= t < s`) -- alone, in chains of
 * limbs, in loops, across branches, with constant operands, with a carry in
 * that is a whole word, and with either result unused.
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

    want = ref_add32((u32)a, (u32)b, (u32)x != (u32)y, &want_c);
    r32 = cmp_add32((u32)a, (u32)b, (u32)x, (u32)y, &o32);
    expect("cmp_add32", r32, want);
    expect("cmp_add32 carry", o32, want_c);
    want = ref_sub32((u32)a, (u32)b, (u32)x == (u32)y, &want_c);
    r32 = cmp_sub32((u32)a, (u32)b, (u32)x, (u32)y, &o32);
    expect("cmp_sub32", r32, want);
    expect("cmp_sub32 borrow", o32, want_c);
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
