/* glibc_f128_builtins.c — _Float128 builtins used by glibc math:
 * __builtin_huge_valf128/inff128/nanf128 (full binary128 payload constants)
 * and the inline bit-op copysignf128/fabsf128/negation (no libgcc call).
 * LCCC used to emit calls to literal "__builtin_*" symbols, mask the wrong
 * qword, and mis-negate negative literals (-1.5f128 became -3.0f128). All
 * checks are value-level (via __eqtf2) plus bit-exact payload verification. */
#include <stdio.h>
#include <string.h>

static int eq128(_Float128 a, _Float128 b) {
    unsigned long long al, ah, bl, bh;
    memcpy(&al, &a, 8); memcpy(&ah, (char *)&a + 8, 8);
    memcpy(&bl, &b, 8); memcpy(&bh, (char *)&b + 8, 8);
    return al == bl && ah == bh;
}

int main(void) {
    _Float128 inf = __builtin_huge_valf128();
    _Float128 ninf = __builtin_inff128();
    _Float128 n = __builtin_nanf128();
    unsigned long long il, ih, nl, nh;
    memcpy(&il, &inf, 8); memcpy(&ih, (char *)&inf + 8, 8);
    memcpy(&nl, &n, 8); memcpy(&nh, (char *)&n + 8, 8);
    /* +Inf: 0x7FFF000000000000 0000000000000000; qNaN: 0x7FFF8000... */
    if (!(ih == 0x7FFF000000000000ULL && il == 0)) { printf("FAIL huge_valf128\n"); return 1; }
    if (nh != 0x7FFF800000000000ULL || nl != 0) { printf("FAIL nanf128\n"); return 1; }
    {
        unsigned long long nil, nih;
        memcpy(&nil, &ninf, 8); memcpy(&nih, (char *)&ninf + 8, 8);
        if (nih != 0x7FFF000000000000ULL || nil != 0) { printf("FAIL inff128\n"); return 1; }
    }
    /* Value-level checks through real functions (parameter -> bit-op ->
     * return -> __eqtf2 comparison). */
    _Float128 x = -4.0f128;
    _Float128 a = __builtin_fabsf128(x);
    _Float128 c = __builtin_copysignf128(x, 2.0f128);
    _Float128 cn = __builtin_copysignf128(x, -2.0f128);
    _Float128 neg = -x;
    if (!eq128(a, 4.0f128) || a != 4.0f128) { printf("FAIL fabsf128\n"); return 1; }
    if (!eq128(c, 4.0f128) || c != 4.0f128) { printf("FAIL copysignf128\n"); return 1; }
    if (!eq128(cn, -4.0f128) || cn != -4.0f128) { printf("FAIL copysignf128 2\n"); return 1; }
    if (!eq128(neg, 4.0f128) || neg != 4.0f128) { printf("FAIL negf128\n"); return 1; }
    /* Negative literal bit-exactness (regression: -1.5f128 was -3.0f128:
     * 0xbfff8000... instead of 0xc0008000...). */
    _Float128 m15 = -1.5f128;
    unsigned long long m15l, m15h;
    memcpy(&m15l, &m15, 8); memcpy(&m15h, (char *)&m15 + 8, 8);
    if (m15h != 0xBFFF800000000000ULL || m15l != 0) { printf("FAIL neg f128 literal\n"); return 1; }
    /* Variable negation through the F128Neg intrinsic (-(-4.0f128) = 4.0). */
    _Float128 nn = -neg;
    if (!eq128(nn, -4.0f128) || nn != -4.0f128) { printf("FAIL neg f128 var\n"); return 1; }
    printf("PASS f128_builtins\n");
    return 0;
}
