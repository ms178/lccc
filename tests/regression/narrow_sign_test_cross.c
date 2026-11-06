/* Sign tests of narrow values on AArch64 and RV64, executed by
 * check_switch_cross_execute.py at every optimization level.
 *
 * `(signed char)c < 0` reads bit 7 of c.  AArch64 folds `sxtb t, s;
 * cmp t, #0` read only by a sign condition into `tst s, #0x80` with the
 * condition turned into `ne`/`eq` (a materialized `lt` into `ubfx s, #7`, a
 * `csetm` of it into `sbfx`); RV64 drops the `srai` of a `slli; srai`
 * extension that only a sign test reads, turns the test of a zero-extended
 * byte into one `srli 7` and a negated byte test into `andi 128; seqz`, and
 * trusts the psABI's extension of `signed char`/`short` arguments, so
 * re-extending one is a move.  A narrow load extended the other way round
 * loads with that signedness on both (`lb`, `ldrsb`), and AArch64 moves the
 * sign bit of a full register into place for `x < 0`, `-(x < 0)` and
 * `c + (x < 0)` (`lsr`, `asr`/`sbfx`, `add ..., lsr #31`) where the flags
 * die at the reader.  Each function is checked against an
 * arithmetic model -- bit operations, never the tested expression -- over
 * every byte value and a spread of halfwords, with arbitrary bits above
 * the narrow value wherever the type lets a caller pass them.
 */
#define NOINLINE __attribute__((noinline))

NOINLINE int fire_lt(int x) { return (unsigned)(signed char)x < 300; }
NOINLINE int fire_le(int x) { return (unsigned)(signed char)x <= 300; }
NOINLINE int fire_ge(int x) { return (unsigned)(signed char)x >= 300; }
NOINLINE int fire_gt(int x) { return (unsigned)(signed char)x > 300; }
NOINLINE int fire16_ge(int x) { return (unsigned)(short)x >= 70000; }

NOINLINE int sgn_ge(signed char x) { return x >= 0; }
NOINLINE int sgn_lt(signed char x) { return x < 0; }
NOINLINE int sgn16_ge(short x) { return x >= 0; }
NOINLINE int sgn16_lt(short x) { return x < 0; }
NOINLINE int sgn_mask(signed char x) { return -(x < 0); }
NOINLINE long sgn_mask_long(short x) { return -(long)(x < 0); }
NOINLINE long widen_s8(signed char c) { return c; }
NOINLINE long widen_s16(short c) { return c; }
NOINLINE unsigned long widen_s8_unsigned(signed char c) { return (unsigned long)(long)c; }

NOINLINE int sel_lt(signed char x, int a, int b) { return x < 0 ? a : b; }
NOINLINE int sel16_ge(short x, int a, int b) { return x >= 0 ? a : b; }
NOINLINE int br_lt(signed char x, int a, int b)
{
    if (x < 0)
        return a + 1;
    return b - 1;
}
NOINLINE int br16_ge(short x, int a, int b)
{
    if (x >= 0)
        return a * 3;
    return b ^ 5;
}

/* The sign of a narrowed sum: the int sum has bits above the byte. */
NOINLINE int sgn_of_sum(signed char a, signed char b) { return (signed char)(a + b) < 0; }
NOINLINE int sgn_of_int(int v) { return (signed char)v < 0; }
NOINLINE int sgn16_of_int(int v) { return (short)v >= 0; }

/* A sign-extended narrow value widened through `unsigned`: never negative as
 * a long, whatever its bit 7 (15) -- a W-form extension zeroes bits 63:32. */
NOINLINE int never_neg(signed char c)
{
    unsigned u = (unsigned)(int)c;
    long l = (long)u;
    return l < 0;
}
NOINLINE long never_neg_sel(short c, long a, long b)
{
    unsigned u = (unsigned)(int)c;
    long l = (long)u;
    return l < 0 ? a : b;
}

/* Narrow loads extended by the other signedness. */
NOINLINE long load_sext8(const unsigned char *p, int i) { return (signed char)p[i]; }
NOINLINE long load_sext16(const unsigned short *p, int i) { return (short)p[i]; }
NOINLINE unsigned long load_zext8(const signed char *p, int i) { return (unsigned char)p[i]; }
NOINLINE unsigned long load_zext16(const short *p, int i) { return (unsigned short)p[i]; }
NOINLINE int load_sign8(const unsigned char *p, int i) { return (signed char)p[i] < 0 ? 5 : 9; }

/* Sign tests of full registers.  A long whose low word is negative as an int
 * (0x80000000) is not negative: the X compare's sign is bit 63. */
NOINLINE int isneg(int x) { return x < 0; }
NOINLINE long isneg_long(long x) { return x < 0; }
NOINLINE int isnonneg(int x) { return x >= 0; }
NOINLINE int negmask(int x) { return -(x < 0); }
NOINLINE long negmask_long(long x) { return -(long)(x < 0); }
NOINLINE long negmask_widened(int x) { return -(long)(x < 0); }
NOINLINE int addneg(int c, int x) { return c + (x < 0); }
NOINLINE long addneg_long(long c, long x) { return c + (x < 0); }
NOINLINE int addnonneg(int c, int x) { return c + (x >= 0); }
NOINLINE int count_neg32(const int *a, int n)
{
    int c = 0;
    for (int i = 0; i < n; i++)
        c += a[i] < 0;
    return c;
}

/* The extended value is read again after the test: the extension stays. */
NOINLINE int keep_ext(int v, int *out)
{
    int t = (signed char)v;
    int neg = t < 0;
    *out = t;
    return neg;
}

/* Loops: an ASCII fast path and counts over bytes and halfwords. */
NOINLINE int ascii_prefix(const char *s, int n)
{
    int i = 0;
    while (i < n && (signed char)s[i] >= 0)
        i++;
    return i;
}
NOINLINE int count_hi(const unsigned char *s, int n)
{
    int c = 0;
    for (int i = 0; i < n; i++)
        if ((signed char)s[i] < 0)
            c++;
    return c;
}
NOINLINE int count_lo(const unsigned char *s, int n)
{
    int c = 0;
    for (int i = 0; i < n; i++)
        c += (signed char)s[i] >= 0;
    return c;
}
NOINLINE int count_neg16(const unsigned short *s, int n)
{
    int c = 0;
    for (int i = 0; i < n; i++)
        c += (short)s[i] < 0;
    return c;
}
NOINLINE long sum_neg_bytes(const signed char *s, int n)
{
    long acc = 0;
    for (int i = 0; i < n; i++)
        if (s[i] < 0)
            acc += s[i];
    return acc;
}

static int bit7(int v) { return (v >> 7) & 1; }
static int bit31(int v) { return (int)((unsigned)v >> 31); }
static long bit63(long v) { return (long)((unsigned long)v >> 63); }
static int bit15(int v) { return (v >> 15) & 1; }
static long sext8(long v) { return (long)((v & 0xff) ^ 0x80) - 0x80; }
static long sext16(long v) { return (long)((v & 0xffff) ^ 0x8000) - 0x8000; }

static unsigned char bytes[300];
static unsigned short halves[300];
static int words[300];

static const long wide_values[] = {
    0, 1, -1, 2, -2, 127, -128, 0x7fffffffL, -0x80000000L, 0x80000000L, 0xffffffffL,
    0x100000000L, -0x100000000L, 0x7fffffffffffffffL, -0x7fffffffffffffffL - 1,
    0x8000000000000000UL >> 1, 0x123456789abcdefL, -0x123456789abcdefL,
};

int main(void)
{
    int bad = 0;
    for (int v = -1024; v < 1024; v++) {
        signed char c = (signed char)v;
        int neg = bit7(v);
        bad += fire_lt(v) != !neg;
        bad += fire_le(v) != !neg;
        bad += fire_ge(v) != neg;
        bad += fire_gt(v) != neg;
        bad += sgn_ge(c) != !neg;
        bad += sgn_lt(c) != neg;
        bad += sgn_mask(c) != -neg;
        bad += widen_s8(c) != sext8(v);
        bad += widen_s8_unsigned(c) != (unsigned long)sext8(v);
        bad += sel_lt(c, v, ~v) != (neg ? v : ~v);
        bad += br_lt(c, v, 3 * v) != (neg ? v + 1 : 3 * v - 1);
        bad += sgn_of_sum(c, (signed char)(v >> 3)) != bit7(v + (int)sext8(v >> 3));
        bad += sgn_of_int(v * 977) != bit7(v * 977);
        int seen = 12345;
        bad += keep_ext(v, &seen) != neg;
        bad += seen != (int)sext8(v);
        bad += never_neg(c) != 0;
    }
    for (int v = -140000; v <= 140000; v += 7) {
        short s = (short)v;
        int neg = bit15(v);
        bad += fire16_ge(v) != neg;
        bad += sgn16_ge(s) != !neg;
        bad += sgn16_lt(s) != neg;
        bad += sgn_mask_long(s) != -(long)neg;
        bad += widen_s16(s) != sext16(v);
        bad += sel16_ge(s, v, v + 9) != (neg ? v + 9 : v);
        bad += br16_ge(s, v, v * 5) != (neg ? (v * 5) ^ 5 : v * 3);
        bad += sgn16_of_int(v * 31) != !bit15(v * 31);
        bad += never_neg_sel(s, v, ~(long)v) != ~(long)v;
    }
    for (unsigned k = 0; k < sizeof wide_values / sizeof wide_values[0]; k++) {
        long w = wide_values[k];
        int x = (int)w;
        bad += isneg(x) != bit31(x);
        bad += isnonneg(x) != !bit31(x);
        bad += negmask(x) != -bit31(x);
        bad += negmask_widened(x) != -(long)bit31(x);
        bad += addneg((int)k, x) != (int)k + bit31(x);
        bad += addnonneg((int)k, x) != (int)k + !bit31(x);
        bad += isneg_long(w) != bit63(w);
        bad += negmask_long(w) != -bit63(w);
        bad += addneg_long(w ^ 5, w) != (w ^ 5) + bit63(w);
    }
    for (int v = -70000; v <= 70000; v += 13) {
        int x = v * 30677;
        bad += isneg(x) != bit31(x);
        bad += negmask(x) != -bit31(x);
        bad += addneg(v, x) != v + bit31(x);
        bad += isneg_long((long)x << 20) != bit63((long)x << 20);
    }
    long want_sum = 0;
    int want_hi = 0, want_neg16 = 0, want_neg32 = 0;
    for (int i = 0; i < 300; i++) {
        bytes[i] = (unsigned char)(i * 37 + (i >> 2));
        halves[i] = (unsigned short)(i * 2741 + 13);
        words[i] = (int)((unsigned)i * 2654435761u);
        want_neg32 += bit31(words[i]);
        want_hi += bit7(bytes[i]);
        want_neg16 += bit15(halves[i]);
        if (bit7(bytes[i]))
            want_sum += sext8(bytes[i]);
    }
    bad += count_hi(bytes, 300) != want_hi;
    bad += count_lo(bytes, 300) != 300 - want_hi;
    bad += count_neg16(halves, 300) != want_neg16;
    bad += count_neg32(words, 300) != want_neg32;
    bad += sum_neg_bytes((const signed char *)bytes, 300) != want_sum;
    for (int i = 0; i < 300; i++) {
        bad += load_sext8(bytes, i) != sext8(bytes[i]);
        bad += load_sext16(halves, i) != sext16(halves[i]);
        bad += load_zext8((const signed char *)bytes, i) != (unsigned long)(bytes[i] & 0xff);
        bad += load_zext16((const short *)halves, i) != (unsigned long)(halves[i] & 0xffff);
        bad += load_sign8(bytes, i) != (bit7(bytes[i]) ? 5 : 9);
    }
    for (int stop = 0; stop < 300; stop += 23) {
        char text[300];
        for (int i = 0; i < 300; i++)
            text[i] = (char)(i < stop ? 'a' + i % 26 : 0x80 | i);
        bad += ascii_prefix(text, 300) != stop;
        bad += ascii_prefix(text, stop / 2) != stop / 2;
    }
    return bad != 0;
}
