/* Byte-sum reductions with COMPILE-TIME trip counts.
 *
 * A constant limit decides both loops of the packed form at compile time:
 * the vector loop is a guard-free do-while (at least one full vector), the
 * scalar remainder starts at the constant packed byte count, and it is
 * elided when the limit is a multiple of the vector width.  The limits
 * straddle every boundary of both widths (16 on the SSE2 baseline, 32 under
 * AVX2: see the _avx2 twin): zero remainders, remainders of one and W-1,
 * the two-vector admission floor, plus large prime and power-of-two trips.
 * Every predicate-count kind (== 0 counts the raw mask, != 0 counts its
 * complement), unsigned and signed sums, and int / unsigned / long IVs are
 * covered; the sieve-shaped loops (`for (i = 2; i <= L; i++)`) reach the
 * same form through the start-rebasing normaliser.  The arrays are globals,
 * so every loop also exercises the in-body symbol-address
 * rematerialisation.  GCC 16.2 is the output oracle.
 */
#include <stdio.h>

#define N 4200
static unsigned char usrc[N];
static signed char ssrc[N];

#define DEF(NAME, ACC, IVT, LIM, EXPR)                                   \
    __attribute__((noinline)) ACC NAME(void) {                          \
        ACC c = 0;                                                      \
        for (IVT i = 0; i < (IVT)(LIM); i++)                            \
            c += (EXPR);                                                \
        return c;                                                       \
    }

#define SIEVE(NAME, LIM)                                                 \
    __attribute__((noinline)) int NAME(void) {                          \
        int c = 0;                                                      \
        for (int i = 2; i <= (LIM); i++)                                \
            if (usrc[i])                                                \
                c++;                                                    \
        return c;                                                       \
    }

DEF(k_eq_i_32, unsigned long, int, 32, usrc[i] == 0)
DEF(k_eq_i_33, unsigned long, int, 33, usrc[i] == 0)
DEF(k_eq_i_40, unsigned long, int, 40, usrc[i] == 0)
DEF(k_eq_i_47, unsigned long, int, 47, usrc[i] == 0)
DEF(k_eq_i_48, unsigned long, int, 48, usrc[i] == 0)
DEF(k_eq_i_49, unsigned long, int, 49, usrc[i] == 0)
DEF(k_eq_i_63, unsigned long, int, 63, usrc[i] == 0)
DEF(k_eq_i_64, unsigned long, int, 64, usrc[i] == 0)
DEF(k_eq_i_65, unsigned long, int, 65, usrc[i] == 0)
DEF(k_eq_i_95, unsigned long, int, 95, usrc[i] == 0)
DEF(k_eq_i_96, unsigned long, int, 96, usrc[i] == 0)
DEF(k_eq_i_97, unsigned long, int, 97, usrc[i] == 0)
DEF(k_eq_i_127, unsigned long, int, 127, usrc[i] == 0)
DEF(k_eq_i_128, unsigned long, int, 128, usrc[i] == 0)
DEF(k_eq_i_129, unsigned long, int, 129, usrc[i] == 0)
DEF(k_eq_i_1031, unsigned long, int, 1031, usrc[i] == 0)
DEF(k_eq_i_4096, unsigned long, int, 4096, usrc[i] == 0)
DEF(k_eq_i_4111, unsigned long, int, 4111, usrc[i] == 0)
DEF(k_eq_u_32, unsigned long, unsigned, 32, usrc[i] == 0)
DEF(k_eq_u_33, unsigned long, unsigned, 33, usrc[i] == 0)
DEF(k_eq_u_40, unsigned long, unsigned, 40, usrc[i] == 0)
DEF(k_eq_u_47, unsigned long, unsigned, 47, usrc[i] == 0)
DEF(k_eq_u_48, unsigned long, unsigned, 48, usrc[i] == 0)
DEF(k_eq_u_49, unsigned long, unsigned, 49, usrc[i] == 0)
DEF(k_eq_u_63, unsigned long, unsigned, 63, usrc[i] == 0)
DEF(k_eq_u_64, unsigned long, unsigned, 64, usrc[i] == 0)
DEF(k_eq_u_65, unsigned long, unsigned, 65, usrc[i] == 0)
DEF(k_eq_u_95, unsigned long, unsigned, 95, usrc[i] == 0)
DEF(k_eq_u_96, unsigned long, unsigned, 96, usrc[i] == 0)
DEF(k_eq_u_97, unsigned long, unsigned, 97, usrc[i] == 0)
DEF(k_eq_u_127, unsigned long, unsigned, 127, usrc[i] == 0)
DEF(k_eq_u_128, unsigned long, unsigned, 128, usrc[i] == 0)
DEF(k_eq_u_129, unsigned long, unsigned, 129, usrc[i] == 0)
DEF(k_eq_u_1031, unsigned long, unsigned, 1031, usrc[i] == 0)
DEF(k_eq_u_4096, unsigned long, unsigned, 4096, usrc[i] == 0)
DEF(k_eq_u_4111, unsigned long, unsigned, 4111, usrc[i] == 0)
DEF(k_eq_l_32, unsigned long, long, 32, usrc[i] == 0)
DEF(k_eq_l_33, unsigned long, long, 33, usrc[i] == 0)
DEF(k_eq_l_40, unsigned long, long, 40, usrc[i] == 0)
DEF(k_eq_l_47, unsigned long, long, 47, usrc[i] == 0)
DEF(k_eq_l_48, unsigned long, long, 48, usrc[i] == 0)
DEF(k_eq_l_49, unsigned long, long, 49, usrc[i] == 0)
DEF(k_eq_l_63, unsigned long, long, 63, usrc[i] == 0)
DEF(k_eq_l_64, unsigned long, long, 64, usrc[i] == 0)
DEF(k_eq_l_65, unsigned long, long, 65, usrc[i] == 0)
DEF(k_eq_l_95, unsigned long, long, 95, usrc[i] == 0)
DEF(k_eq_l_96, unsigned long, long, 96, usrc[i] == 0)
DEF(k_eq_l_97, unsigned long, long, 97, usrc[i] == 0)
DEF(k_eq_l_127, unsigned long, long, 127, usrc[i] == 0)
DEF(k_eq_l_128, unsigned long, long, 128, usrc[i] == 0)
DEF(k_eq_l_129, unsigned long, long, 129, usrc[i] == 0)
DEF(k_eq_l_1031, unsigned long, long, 1031, usrc[i] == 0)
DEF(k_eq_l_4096, unsigned long, long, 4096, usrc[i] == 0)
DEF(k_eq_l_4111, unsigned long, long, 4111, usrc[i] == 0)
DEF(k_ne_i_32, unsigned long, int, 32, usrc[i] != 0)
DEF(k_ne_i_33, unsigned long, int, 33, usrc[i] != 0)
DEF(k_ne_i_40, unsigned long, int, 40, usrc[i] != 0)
DEF(k_ne_i_47, unsigned long, int, 47, usrc[i] != 0)
DEF(k_ne_i_48, unsigned long, int, 48, usrc[i] != 0)
DEF(k_ne_i_49, unsigned long, int, 49, usrc[i] != 0)
DEF(k_ne_i_63, unsigned long, int, 63, usrc[i] != 0)
DEF(k_ne_i_64, unsigned long, int, 64, usrc[i] != 0)
DEF(k_ne_i_65, unsigned long, int, 65, usrc[i] != 0)
DEF(k_ne_i_95, unsigned long, int, 95, usrc[i] != 0)
DEF(k_ne_i_96, unsigned long, int, 96, usrc[i] != 0)
DEF(k_ne_i_97, unsigned long, int, 97, usrc[i] != 0)
DEF(k_ne_i_127, unsigned long, int, 127, usrc[i] != 0)
DEF(k_ne_i_128, unsigned long, int, 128, usrc[i] != 0)
DEF(k_ne_i_129, unsigned long, int, 129, usrc[i] != 0)
DEF(k_ne_i_1031, unsigned long, int, 1031, usrc[i] != 0)
DEF(k_ne_i_4096, unsigned long, int, 4096, usrc[i] != 0)
DEF(k_ne_i_4111, unsigned long, int, 4111, usrc[i] != 0)
DEF(k_ne_u_32, unsigned long, unsigned, 32, usrc[i] != 0)
DEF(k_ne_u_33, unsigned long, unsigned, 33, usrc[i] != 0)
DEF(k_ne_u_40, unsigned long, unsigned, 40, usrc[i] != 0)
DEF(k_ne_u_47, unsigned long, unsigned, 47, usrc[i] != 0)
DEF(k_ne_u_48, unsigned long, unsigned, 48, usrc[i] != 0)
DEF(k_ne_u_49, unsigned long, unsigned, 49, usrc[i] != 0)
DEF(k_ne_u_63, unsigned long, unsigned, 63, usrc[i] != 0)
DEF(k_ne_u_64, unsigned long, unsigned, 64, usrc[i] != 0)
DEF(k_ne_u_65, unsigned long, unsigned, 65, usrc[i] != 0)
DEF(k_ne_u_95, unsigned long, unsigned, 95, usrc[i] != 0)
DEF(k_ne_u_96, unsigned long, unsigned, 96, usrc[i] != 0)
DEF(k_ne_u_97, unsigned long, unsigned, 97, usrc[i] != 0)
DEF(k_ne_u_127, unsigned long, unsigned, 127, usrc[i] != 0)
DEF(k_ne_u_128, unsigned long, unsigned, 128, usrc[i] != 0)
DEF(k_ne_u_129, unsigned long, unsigned, 129, usrc[i] != 0)
DEF(k_ne_u_1031, unsigned long, unsigned, 1031, usrc[i] != 0)
DEF(k_ne_u_4096, unsigned long, unsigned, 4096, usrc[i] != 0)
DEF(k_ne_u_4111, unsigned long, unsigned, 4111, usrc[i] != 0)
DEF(k_ne_l_32, unsigned long, long, 32, usrc[i] != 0)
DEF(k_ne_l_33, unsigned long, long, 33, usrc[i] != 0)
DEF(k_ne_l_40, unsigned long, long, 40, usrc[i] != 0)
DEF(k_ne_l_47, unsigned long, long, 47, usrc[i] != 0)
DEF(k_ne_l_48, unsigned long, long, 48, usrc[i] != 0)
DEF(k_ne_l_49, unsigned long, long, 49, usrc[i] != 0)
DEF(k_ne_l_63, unsigned long, long, 63, usrc[i] != 0)
DEF(k_ne_l_64, unsigned long, long, 64, usrc[i] != 0)
DEF(k_ne_l_65, unsigned long, long, 65, usrc[i] != 0)
DEF(k_ne_l_95, unsigned long, long, 95, usrc[i] != 0)
DEF(k_ne_l_96, unsigned long, long, 96, usrc[i] != 0)
DEF(k_ne_l_97, unsigned long, long, 97, usrc[i] != 0)
DEF(k_ne_l_127, unsigned long, long, 127, usrc[i] != 0)
DEF(k_ne_l_128, unsigned long, long, 128, usrc[i] != 0)
DEF(k_ne_l_129, unsigned long, long, 129, usrc[i] != 0)
DEF(k_ne_l_1031, unsigned long, long, 1031, usrc[i] != 0)
DEF(k_ne_l_4096, unsigned long, long, 4096, usrc[i] != 0)
DEF(k_ne_l_4111, unsigned long, long, 4111, usrc[i] != 0)
DEF(k_usum_i_32, unsigned long, int, 32, usrc[i])
DEF(k_usum_i_33, unsigned long, int, 33, usrc[i])
DEF(k_usum_i_40, unsigned long, int, 40, usrc[i])
DEF(k_usum_i_47, unsigned long, int, 47, usrc[i])
DEF(k_usum_i_48, unsigned long, int, 48, usrc[i])
DEF(k_usum_i_49, unsigned long, int, 49, usrc[i])
DEF(k_usum_i_63, unsigned long, int, 63, usrc[i])
DEF(k_usum_i_64, unsigned long, int, 64, usrc[i])
DEF(k_usum_i_65, unsigned long, int, 65, usrc[i])
DEF(k_usum_i_95, unsigned long, int, 95, usrc[i])
DEF(k_usum_i_96, unsigned long, int, 96, usrc[i])
DEF(k_usum_i_97, unsigned long, int, 97, usrc[i])
DEF(k_usum_i_127, unsigned long, int, 127, usrc[i])
DEF(k_usum_i_128, unsigned long, int, 128, usrc[i])
DEF(k_usum_i_129, unsigned long, int, 129, usrc[i])
DEF(k_usum_i_1031, unsigned long, int, 1031, usrc[i])
DEF(k_usum_i_4096, unsigned long, int, 4096, usrc[i])
DEF(k_usum_i_4111, unsigned long, int, 4111, usrc[i])
DEF(k_usum_u_32, unsigned long, unsigned, 32, usrc[i])
DEF(k_usum_u_33, unsigned long, unsigned, 33, usrc[i])
DEF(k_usum_u_40, unsigned long, unsigned, 40, usrc[i])
DEF(k_usum_u_47, unsigned long, unsigned, 47, usrc[i])
DEF(k_usum_u_48, unsigned long, unsigned, 48, usrc[i])
DEF(k_usum_u_49, unsigned long, unsigned, 49, usrc[i])
DEF(k_usum_u_63, unsigned long, unsigned, 63, usrc[i])
DEF(k_usum_u_64, unsigned long, unsigned, 64, usrc[i])
DEF(k_usum_u_65, unsigned long, unsigned, 65, usrc[i])
DEF(k_usum_u_95, unsigned long, unsigned, 95, usrc[i])
DEF(k_usum_u_96, unsigned long, unsigned, 96, usrc[i])
DEF(k_usum_u_97, unsigned long, unsigned, 97, usrc[i])
DEF(k_usum_u_127, unsigned long, unsigned, 127, usrc[i])
DEF(k_usum_u_128, unsigned long, unsigned, 128, usrc[i])
DEF(k_usum_u_129, unsigned long, unsigned, 129, usrc[i])
DEF(k_usum_u_1031, unsigned long, unsigned, 1031, usrc[i])
DEF(k_usum_u_4096, unsigned long, unsigned, 4096, usrc[i])
DEF(k_usum_u_4111, unsigned long, unsigned, 4111, usrc[i])
DEF(k_usum_l_32, unsigned long, long, 32, usrc[i])
DEF(k_usum_l_33, unsigned long, long, 33, usrc[i])
DEF(k_usum_l_40, unsigned long, long, 40, usrc[i])
DEF(k_usum_l_47, unsigned long, long, 47, usrc[i])
DEF(k_usum_l_48, unsigned long, long, 48, usrc[i])
DEF(k_usum_l_49, unsigned long, long, 49, usrc[i])
DEF(k_usum_l_63, unsigned long, long, 63, usrc[i])
DEF(k_usum_l_64, unsigned long, long, 64, usrc[i])
DEF(k_usum_l_65, unsigned long, long, 65, usrc[i])
DEF(k_usum_l_95, unsigned long, long, 95, usrc[i])
DEF(k_usum_l_96, unsigned long, long, 96, usrc[i])
DEF(k_usum_l_97, unsigned long, long, 97, usrc[i])
DEF(k_usum_l_127, unsigned long, long, 127, usrc[i])
DEF(k_usum_l_128, unsigned long, long, 128, usrc[i])
DEF(k_usum_l_129, unsigned long, long, 129, usrc[i])
DEF(k_usum_l_1031, unsigned long, long, 1031, usrc[i])
DEF(k_usum_l_4096, unsigned long, long, 4096, usrc[i])
DEF(k_usum_l_4111, unsigned long, long, 4111, usrc[i])
DEF(k_ssum_i_32, long, int, 32, ssrc[i])
DEF(k_ssum_i_33, long, int, 33, ssrc[i])
DEF(k_ssum_i_40, long, int, 40, ssrc[i])
DEF(k_ssum_i_47, long, int, 47, ssrc[i])
DEF(k_ssum_i_48, long, int, 48, ssrc[i])
DEF(k_ssum_i_49, long, int, 49, ssrc[i])
DEF(k_ssum_i_63, long, int, 63, ssrc[i])
DEF(k_ssum_i_64, long, int, 64, ssrc[i])
DEF(k_ssum_i_65, long, int, 65, ssrc[i])
DEF(k_ssum_i_95, long, int, 95, ssrc[i])
DEF(k_ssum_i_96, long, int, 96, ssrc[i])
DEF(k_ssum_i_97, long, int, 97, ssrc[i])
DEF(k_ssum_i_127, long, int, 127, ssrc[i])
DEF(k_ssum_i_128, long, int, 128, ssrc[i])
DEF(k_ssum_i_129, long, int, 129, ssrc[i])
DEF(k_ssum_i_1031, long, int, 1031, ssrc[i])
DEF(k_ssum_i_4096, long, int, 4096, ssrc[i])
DEF(k_ssum_i_4111, long, int, 4111, ssrc[i])
DEF(k_ssum_u_32, long, unsigned, 32, ssrc[i])
DEF(k_ssum_u_33, long, unsigned, 33, ssrc[i])
DEF(k_ssum_u_40, long, unsigned, 40, ssrc[i])
DEF(k_ssum_u_47, long, unsigned, 47, ssrc[i])
DEF(k_ssum_u_48, long, unsigned, 48, ssrc[i])
DEF(k_ssum_u_49, long, unsigned, 49, ssrc[i])
DEF(k_ssum_u_63, long, unsigned, 63, ssrc[i])
DEF(k_ssum_u_64, long, unsigned, 64, ssrc[i])
DEF(k_ssum_u_65, long, unsigned, 65, ssrc[i])
DEF(k_ssum_u_95, long, unsigned, 95, ssrc[i])
DEF(k_ssum_u_96, long, unsigned, 96, ssrc[i])
DEF(k_ssum_u_97, long, unsigned, 97, ssrc[i])
DEF(k_ssum_u_127, long, unsigned, 127, ssrc[i])
DEF(k_ssum_u_128, long, unsigned, 128, ssrc[i])
DEF(k_ssum_u_129, long, unsigned, 129, ssrc[i])
DEF(k_ssum_u_1031, long, unsigned, 1031, ssrc[i])
DEF(k_ssum_u_4096, long, unsigned, 4096, ssrc[i])
DEF(k_ssum_u_4111, long, unsigned, 4111, ssrc[i])
DEF(k_ssum_l_32, long, long, 32, ssrc[i])
DEF(k_ssum_l_33, long, long, 33, ssrc[i])
DEF(k_ssum_l_40, long, long, 40, ssrc[i])
DEF(k_ssum_l_47, long, long, 47, ssrc[i])
DEF(k_ssum_l_48, long, long, 48, ssrc[i])
DEF(k_ssum_l_49, long, long, 49, ssrc[i])
DEF(k_ssum_l_63, long, long, 63, ssrc[i])
DEF(k_ssum_l_64, long, long, 64, ssrc[i])
DEF(k_ssum_l_65, long, long, 65, ssrc[i])
DEF(k_ssum_l_95, long, long, 95, ssrc[i])
DEF(k_ssum_l_96, long, long, 96, ssrc[i])
DEF(k_ssum_l_97, long, long, 97, ssrc[i])
DEF(k_ssum_l_127, long, long, 127, ssrc[i])
DEF(k_ssum_l_128, long, long, 128, ssrc[i])
DEF(k_ssum_l_129, long, long, 129, ssrc[i])
DEF(k_ssum_l_1031, long, long, 1031, ssrc[i])
DEF(k_ssum_l_4096, long, long, 4096, ssrc[i])
DEF(k_ssum_l_4111, long, long, 4111, ssrc[i])
SIEVE(s_sieve_33, 33)
SIEVE(s_sieve_34, 34)
SIEVE(s_sieve_49, 49)
SIEVE(s_sieve_50, 50)
SIEVE(s_sieve_65, 65)
SIEVE(s_sieve_66, 66)
SIEVE(s_sieve_1032, 1032)
SIEVE(s_sieve_4097, 4097)
SIEVE(s_sieve_4112, 4112)

int main(void) {
    unsigned x = 0x2545F491u;
    for (int round = 0; round < 3; round++) {
        for (int i = 0; i < N; i++) {
            x = x * 1103515245u + 12345u;
            unsigned b = (x >> 16) & 0xFFu;
            /* Round 0: dense zeros; round 1: sparse; round 2: none. */
            if (round == 0 && (b & 3) == 0) b = 0;
            if (round == 2 && b == 0) b = 0x80;
            usrc[i] = (unsigned char)b;
            ssrc[i] = (signed char)(b ^ 0x5A);
        }

        printf("k_eq_i_32 %lu\n", k_eq_i_32());
        printf("k_eq_i_33 %lu\n", k_eq_i_33());
        printf("k_eq_i_40 %lu\n", k_eq_i_40());
        printf("k_eq_i_47 %lu\n", k_eq_i_47());
        printf("k_eq_i_48 %lu\n", k_eq_i_48());
        printf("k_eq_i_49 %lu\n", k_eq_i_49());
        printf("k_eq_i_63 %lu\n", k_eq_i_63());
        printf("k_eq_i_64 %lu\n", k_eq_i_64());
        printf("k_eq_i_65 %lu\n", k_eq_i_65());
        printf("k_eq_i_95 %lu\n", k_eq_i_95());
        printf("k_eq_i_96 %lu\n", k_eq_i_96());
        printf("k_eq_i_97 %lu\n", k_eq_i_97());
        printf("k_eq_i_127 %lu\n", k_eq_i_127());
        printf("k_eq_i_128 %lu\n", k_eq_i_128());
        printf("k_eq_i_129 %lu\n", k_eq_i_129());
        printf("k_eq_i_1031 %lu\n", k_eq_i_1031());
        printf("k_eq_i_4096 %lu\n", k_eq_i_4096());
        printf("k_eq_i_4111 %lu\n", k_eq_i_4111());
        printf("k_eq_u_32 %lu\n", k_eq_u_32());
        printf("k_eq_u_33 %lu\n", k_eq_u_33());
        printf("k_eq_u_40 %lu\n", k_eq_u_40());
        printf("k_eq_u_47 %lu\n", k_eq_u_47());
        printf("k_eq_u_48 %lu\n", k_eq_u_48());
        printf("k_eq_u_49 %lu\n", k_eq_u_49());
        printf("k_eq_u_63 %lu\n", k_eq_u_63());
        printf("k_eq_u_64 %lu\n", k_eq_u_64());
        printf("k_eq_u_65 %lu\n", k_eq_u_65());
        printf("k_eq_u_95 %lu\n", k_eq_u_95());
        printf("k_eq_u_96 %lu\n", k_eq_u_96());
        printf("k_eq_u_97 %lu\n", k_eq_u_97());
        printf("k_eq_u_127 %lu\n", k_eq_u_127());
        printf("k_eq_u_128 %lu\n", k_eq_u_128());
        printf("k_eq_u_129 %lu\n", k_eq_u_129());
        printf("k_eq_u_1031 %lu\n", k_eq_u_1031());
        printf("k_eq_u_4096 %lu\n", k_eq_u_4096());
        printf("k_eq_u_4111 %lu\n", k_eq_u_4111());
        printf("k_eq_l_32 %lu\n", k_eq_l_32());
        printf("k_eq_l_33 %lu\n", k_eq_l_33());
        printf("k_eq_l_40 %lu\n", k_eq_l_40());
        printf("k_eq_l_47 %lu\n", k_eq_l_47());
        printf("k_eq_l_48 %lu\n", k_eq_l_48());
        printf("k_eq_l_49 %lu\n", k_eq_l_49());
        printf("k_eq_l_63 %lu\n", k_eq_l_63());
        printf("k_eq_l_64 %lu\n", k_eq_l_64());
        printf("k_eq_l_65 %lu\n", k_eq_l_65());
        printf("k_eq_l_95 %lu\n", k_eq_l_95());
        printf("k_eq_l_96 %lu\n", k_eq_l_96());
        printf("k_eq_l_97 %lu\n", k_eq_l_97());
        printf("k_eq_l_127 %lu\n", k_eq_l_127());
        printf("k_eq_l_128 %lu\n", k_eq_l_128());
        printf("k_eq_l_129 %lu\n", k_eq_l_129());
        printf("k_eq_l_1031 %lu\n", k_eq_l_1031());
        printf("k_eq_l_4096 %lu\n", k_eq_l_4096());
        printf("k_eq_l_4111 %lu\n", k_eq_l_4111());
        printf("k_ne_i_32 %lu\n", k_ne_i_32());
        printf("k_ne_i_33 %lu\n", k_ne_i_33());
        printf("k_ne_i_40 %lu\n", k_ne_i_40());
        printf("k_ne_i_47 %lu\n", k_ne_i_47());
        printf("k_ne_i_48 %lu\n", k_ne_i_48());
        printf("k_ne_i_49 %lu\n", k_ne_i_49());
        printf("k_ne_i_63 %lu\n", k_ne_i_63());
        printf("k_ne_i_64 %lu\n", k_ne_i_64());
        printf("k_ne_i_65 %lu\n", k_ne_i_65());
        printf("k_ne_i_95 %lu\n", k_ne_i_95());
        printf("k_ne_i_96 %lu\n", k_ne_i_96());
        printf("k_ne_i_97 %lu\n", k_ne_i_97());
        printf("k_ne_i_127 %lu\n", k_ne_i_127());
        printf("k_ne_i_128 %lu\n", k_ne_i_128());
        printf("k_ne_i_129 %lu\n", k_ne_i_129());
        printf("k_ne_i_1031 %lu\n", k_ne_i_1031());
        printf("k_ne_i_4096 %lu\n", k_ne_i_4096());
        printf("k_ne_i_4111 %lu\n", k_ne_i_4111());
        printf("k_ne_u_32 %lu\n", k_ne_u_32());
        printf("k_ne_u_33 %lu\n", k_ne_u_33());
        printf("k_ne_u_40 %lu\n", k_ne_u_40());
        printf("k_ne_u_47 %lu\n", k_ne_u_47());
        printf("k_ne_u_48 %lu\n", k_ne_u_48());
        printf("k_ne_u_49 %lu\n", k_ne_u_49());
        printf("k_ne_u_63 %lu\n", k_ne_u_63());
        printf("k_ne_u_64 %lu\n", k_ne_u_64());
        printf("k_ne_u_65 %lu\n", k_ne_u_65());
        printf("k_ne_u_95 %lu\n", k_ne_u_95());
        printf("k_ne_u_96 %lu\n", k_ne_u_96());
        printf("k_ne_u_97 %lu\n", k_ne_u_97());
        printf("k_ne_u_127 %lu\n", k_ne_u_127());
        printf("k_ne_u_128 %lu\n", k_ne_u_128());
        printf("k_ne_u_129 %lu\n", k_ne_u_129());
        printf("k_ne_u_1031 %lu\n", k_ne_u_1031());
        printf("k_ne_u_4096 %lu\n", k_ne_u_4096());
        printf("k_ne_u_4111 %lu\n", k_ne_u_4111());
        printf("k_ne_l_32 %lu\n", k_ne_l_32());
        printf("k_ne_l_33 %lu\n", k_ne_l_33());
        printf("k_ne_l_40 %lu\n", k_ne_l_40());
        printf("k_ne_l_47 %lu\n", k_ne_l_47());
        printf("k_ne_l_48 %lu\n", k_ne_l_48());
        printf("k_ne_l_49 %lu\n", k_ne_l_49());
        printf("k_ne_l_63 %lu\n", k_ne_l_63());
        printf("k_ne_l_64 %lu\n", k_ne_l_64());
        printf("k_ne_l_65 %lu\n", k_ne_l_65());
        printf("k_ne_l_95 %lu\n", k_ne_l_95());
        printf("k_ne_l_96 %lu\n", k_ne_l_96());
        printf("k_ne_l_97 %lu\n", k_ne_l_97());
        printf("k_ne_l_127 %lu\n", k_ne_l_127());
        printf("k_ne_l_128 %lu\n", k_ne_l_128());
        printf("k_ne_l_129 %lu\n", k_ne_l_129());
        printf("k_ne_l_1031 %lu\n", k_ne_l_1031());
        printf("k_ne_l_4096 %lu\n", k_ne_l_4096());
        printf("k_ne_l_4111 %lu\n", k_ne_l_4111());
        printf("k_usum_i_32 %lu\n", k_usum_i_32());
        printf("k_usum_i_33 %lu\n", k_usum_i_33());
        printf("k_usum_i_40 %lu\n", k_usum_i_40());
        printf("k_usum_i_47 %lu\n", k_usum_i_47());
        printf("k_usum_i_48 %lu\n", k_usum_i_48());
        printf("k_usum_i_49 %lu\n", k_usum_i_49());
        printf("k_usum_i_63 %lu\n", k_usum_i_63());
        printf("k_usum_i_64 %lu\n", k_usum_i_64());
        printf("k_usum_i_65 %lu\n", k_usum_i_65());
        printf("k_usum_i_95 %lu\n", k_usum_i_95());
        printf("k_usum_i_96 %lu\n", k_usum_i_96());
        printf("k_usum_i_97 %lu\n", k_usum_i_97());
        printf("k_usum_i_127 %lu\n", k_usum_i_127());
        printf("k_usum_i_128 %lu\n", k_usum_i_128());
        printf("k_usum_i_129 %lu\n", k_usum_i_129());
        printf("k_usum_i_1031 %lu\n", k_usum_i_1031());
        printf("k_usum_i_4096 %lu\n", k_usum_i_4096());
        printf("k_usum_i_4111 %lu\n", k_usum_i_4111());
        printf("k_usum_u_32 %lu\n", k_usum_u_32());
        printf("k_usum_u_33 %lu\n", k_usum_u_33());
        printf("k_usum_u_40 %lu\n", k_usum_u_40());
        printf("k_usum_u_47 %lu\n", k_usum_u_47());
        printf("k_usum_u_48 %lu\n", k_usum_u_48());
        printf("k_usum_u_49 %lu\n", k_usum_u_49());
        printf("k_usum_u_63 %lu\n", k_usum_u_63());
        printf("k_usum_u_64 %lu\n", k_usum_u_64());
        printf("k_usum_u_65 %lu\n", k_usum_u_65());
        printf("k_usum_u_95 %lu\n", k_usum_u_95());
        printf("k_usum_u_96 %lu\n", k_usum_u_96());
        printf("k_usum_u_97 %lu\n", k_usum_u_97());
        printf("k_usum_u_127 %lu\n", k_usum_u_127());
        printf("k_usum_u_128 %lu\n", k_usum_u_128());
        printf("k_usum_u_129 %lu\n", k_usum_u_129());
        printf("k_usum_u_1031 %lu\n", k_usum_u_1031());
        printf("k_usum_u_4096 %lu\n", k_usum_u_4096());
        printf("k_usum_u_4111 %lu\n", k_usum_u_4111());
        printf("k_usum_l_32 %lu\n", k_usum_l_32());
        printf("k_usum_l_33 %lu\n", k_usum_l_33());
        printf("k_usum_l_40 %lu\n", k_usum_l_40());
        printf("k_usum_l_47 %lu\n", k_usum_l_47());
        printf("k_usum_l_48 %lu\n", k_usum_l_48());
        printf("k_usum_l_49 %lu\n", k_usum_l_49());
        printf("k_usum_l_63 %lu\n", k_usum_l_63());
        printf("k_usum_l_64 %lu\n", k_usum_l_64());
        printf("k_usum_l_65 %lu\n", k_usum_l_65());
        printf("k_usum_l_95 %lu\n", k_usum_l_95());
        printf("k_usum_l_96 %lu\n", k_usum_l_96());
        printf("k_usum_l_97 %lu\n", k_usum_l_97());
        printf("k_usum_l_127 %lu\n", k_usum_l_127());
        printf("k_usum_l_128 %lu\n", k_usum_l_128());
        printf("k_usum_l_129 %lu\n", k_usum_l_129());
        printf("k_usum_l_1031 %lu\n", k_usum_l_1031());
        printf("k_usum_l_4096 %lu\n", k_usum_l_4096());
        printf("k_usum_l_4111 %lu\n", k_usum_l_4111());
        printf("k_ssum_i_32 %ld\n", k_ssum_i_32());
        printf("k_ssum_i_33 %ld\n", k_ssum_i_33());
        printf("k_ssum_i_40 %ld\n", k_ssum_i_40());
        printf("k_ssum_i_47 %ld\n", k_ssum_i_47());
        printf("k_ssum_i_48 %ld\n", k_ssum_i_48());
        printf("k_ssum_i_49 %ld\n", k_ssum_i_49());
        printf("k_ssum_i_63 %ld\n", k_ssum_i_63());
        printf("k_ssum_i_64 %ld\n", k_ssum_i_64());
        printf("k_ssum_i_65 %ld\n", k_ssum_i_65());
        printf("k_ssum_i_95 %ld\n", k_ssum_i_95());
        printf("k_ssum_i_96 %ld\n", k_ssum_i_96());
        printf("k_ssum_i_97 %ld\n", k_ssum_i_97());
        printf("k_ssum_i_127 %ld\n", k_ssum_i_127());
        printf("k_ssum_i_128 %ld\n", k_ssum_i_128());
        printf("k_ssum_i_129 %ld\n", k_ssum_i_129());
        printf("k_ssum_i_1031 %ld\n", k_ssum_i_1031());
        printf("k_ssum_i_4096 %ld\n", k_ssum_i_4096());
        printf("k_ssum_i_4111 %ld\n", k_ssum_i_4111());
        printf("k_ssum_u_32 %ld\n", k_ssum_u_32());
        printf("k_ssum_u_33 %ld\n", k_ssum_u_33());
        printf("k_ssum_u_40 %ld\n", k_ssum_u_40());
        printf("k_ssum_u_47 %ld\n", k_ssum_u_47());
        printf("k_ssum_u_48 %ld\n", k_ssum_u_48());
        printf("k_ssum_u_49 %ld\n", k_ssum_u_49());
        printf("k_ssum_u_63 %ld\n", k_ssum_u_63());
        printf("k_ssum_u_64 %ld\n", k_ssum_u_64());
        printf("k_ssum_u_65 %ld\n", k_ssum_u_65());
        printf("k_ssum_u_95 %ld\n", k_ssum_u_95());
        printf("k_ssum_u_96 %ld\n", k_ssum_u_96());
        printf("k_ssum_u_97 %ld\n", k_ssum_u_97());
        printf("k_ssum_u_127 %ld\n", k_ssum_u_127());
        printf("k_ssum_u_128 %ld\n", k_ssum_u_128());
        printf("k_ssum_u_129 %ld\n", k_ssum_u_129());
        printf("k_ssum_u_1031 %ld\n", k_ssum_u_1031());
        printf("k_ssum_u_4096 %ld\n", k_ssum_u_4096());
        printf("k_ssum_u_4111 %ld\n", k_ssum_u_4111());
        printf("k_ssum_l_32 %ld\n", k_ssum_l_32());
        printf("k_ssum_l_33 %ld\n", k_ssum_l_33());
        printf("k_ssum_l_40 %ld\n", k_ssum_l_40());
        printf("k_ssum_l_47 %ld\n", k_ssum_l_47());
        printf("k_ssum_l_48 %ld\n", k_ssum_l_48());
        printf("k_ssum_l_49 %ld\n", k_ssum_l_49());
        printf("k_ssum_l_63 %ld\n", k_ssum_l_63());
        printf("k_ssum_l_64 %ld\n", k_ssum_l_64());
        printf("k_ssum_l_65 %ld\n", k_ssum_l_65());
        printf("k_ssum_l_95 %ld\n", k_ssum_l_95());
        printf("k_ssum_l_96 %ld\n", k_ssum_l_96());
        printf("k_ssum_l_97 %ld\n", k_ssum_l_97());
        printf("k_ssum_l_127 %ld\n", k_ssum_l_127());
        printf("k_ssum_l_128 %ld\n", k_ssum_l_128());
        printf("k_ssum_l_129 %ld\n", k_ssum_l_129());
        printf("k_ssum_l_1031 %ld\n", k_ssum_l_1031());
        printf("k_ssum_l_4096 %ld\n", k_ssum_l_4096());
        printf("k_ssum_l_4111 %ld\n", k_ssum_l_4111());
        printf("s_sieve_33 %d\n", s_sieve_33());
        printf("s_sieve_34 %d\n", s_sieve_34());
        printf("s_sieve_49 %d\n", s_sieve_49());
        printf("s_sieve_50 %d\n", s_sieve_50());
        printf("s_sieve_65 %d\n", s_sieve_65());
        printf("s_sieve_66 %d\n", s_sieve_66());
        printf("s_sieve_1032 %d\n", s_sieve_1032());
        printf("s_sieve_4097 %d\n", s_sieve_4097());
        printf("s_sieve_4112 %d\n", s_sieve_4112());
    }
    return 0;
}
