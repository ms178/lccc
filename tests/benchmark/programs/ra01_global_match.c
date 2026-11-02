/*
 * Workload-derived kernel: the access shape of GNU gzip 1.14's
 * `deflate.c:longest_match` (RA-01).
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * No upstream lines are copied: this is a standalone kernel written from the
 * *access shape* that gzip's match search presents to the compiler, so the
 * measurement stays honest about provenance (see WORKLOAD_PROVENANCE.md).
 *
 * The shape, in gzip's terms:
 *   - file-scope `window` (byte array) and `prev` (16-bit chain array),
 *     plus the `strstart` / `prev_length` / `max_chain_length` /
 *     `good_match` / `match_start` scalars;
 *   - a chain walk `cur_match = prev[cur_match & WMASK]` guarded by a
 *     distance limit and a chain budget;
 *   - the two probe compares at `match[best]` / `match[best - 1]` before the
 *     full compare, then a 258-byte extension loop.
 *
 * That access pattern is exactly what makes one machine instruction of
 * difference visible: every iteration addresses `window + cur_match` and
 * `prev + (cur_match & WMASK)` through a register, so a compiler that can
 * fold the symbol into the addressing mode saves the address materialisation
 * in the hottest block, and one that cannot pays for it on every link of the
 * chain.  RA-01 measured GCC never materialising `window+cur_match`.
 *
 * The harness is deterministic and prints one checksum to stdout; the gate
 * `scripts/check_benchmark_outputs.sh` compares that stdout against GCC at
 * -O0..-O3, and `scripts/callgrind_ab.py` uses the same binary for the
 * instruction-count A/B (never as a wall-clock claim).
 */
#include <stdio.h>

typedef unsigned char u8;
typedef unsigned short u16;

u8 window[65536];
u16 prev[32768];
unsigned strstart;
unsigned prev_length;
unsigned max_chain_length;
unsigned good_match;
unsigned match_start;

unsigned
global_match_probe(unsigned cur_match)
{
    unsigned chain = max_chain_length;
    u8 *scan = window + strstart;
    unsigned best = prev_length;
    unsigned limit = strstart > 32506 ? strstart - 32506 : 0;
    u8 end0 = scan[best - 1];
    u8 end1 = scan[best];

    if (best >= good_match)
        chain >>= 2;
    do {
        u8 *match = window + cur_match;
        if (match[best] == end1 && match[best - 1] == end0 && match[0] == scan[0]
            && match[1] == scan[1]) {
            unsigned len = 2;
            while (len < 258 && match[len] == scan[len])
                ++len;
            if (len > best) {
                match_start = cur_match;
                best = len;
            }
        }
        cur_match = prev[cur_match & 32767];
    } while (cur_match > limit && --chain != 0);
    return best;
}

/* Deterministic driver: a repetitive window with long enough runs that the
 * match extension loop executes, and a chain list that descends so the walk
 * terminates.  Everything here is defined behaviour (all indices are in
 * range by construction), so GCC and LCCC must print the same checksum. */
int
main(void)
{
    unsigned i, iter;
    unsigned long acc = 0;

    for (i = 0; i < 65536; i++)
        window[i] = (u8)((i * 37u) ^ (i >> 3) ^ (i & 0x3fu ? 0 : 0xa5u));

    /* prev[p] points strictly back, by 1..17 positions: the chain walk is
     * long (it is throttled by max_chain_length, not by the list), and every
     * link lands inside the section. */
    prev[0] = 0;
    for (i = 1; i < 32768; i++)
        prev[i] = (u16)(i >= 18 ? i - 1 - (i % 17) : 0);

    max_chain_length = 64;
    good_match = 24;

    for (iter = 0; iter < 20000; iter++) {
        unsigned start = 40000u + (iter % 24000u);
        strstart = start;
        prev_length = 3u + (iter % 6u);
        match_start = 0;
        acc += global_match_probe(prev[start & 32767]);
        acc = acc * 31u + match_start;
    }

    printf("%lu\n", acc);
    return 0;
}
