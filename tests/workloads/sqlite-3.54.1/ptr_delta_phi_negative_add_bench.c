/*
 * Host-VM A/B screen for the GEP/Add-encoded backward step. This is distinct
 * from the Sub(base, 7) negative control: the original fold still accepted
 * Add(base, -7), and 31 interleaved pairs exposed a runtime regression.
 * Each executable checks 4096 pointer/value cases before the timed loop;
 * scripts/perf_ab.py also compares full stdout in every A/B sample.
 */
#include <stdio.h>
#include "ptr_delta_phi_negative_add.c"

#define CORPUS_SIZE 4096U
#ifndef BENCH_PASSES
#define BENCH_PASSES 15000U
#endif

static u8 data[2048];
static u8 choices[CORPUS_SIZE];

int main(void) {
    unsigned i, pass, state = 0x20ce271cU;
    unsigned long long checksum = 0;
    for (i = 0; i < 2048; ++i) {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        data[i] = (u8)state;
    }
    for (i = 0; i < CORPUS_SIZE; ++i) {
        const u8 *p = data + 16 + (i & 1023U);
        long want;
        volatile u8 out = 0;
        choices[i] = (u8)(i % 3U);
        want = choices[i] == 1 ? -7 : choices[i] == 2 ? 11 : 0;
        if (guarded_negative_add_delta(p, choices[i], &out) != want || out != p[want]) {
            fputs("negative-add pointer delta oracle failed\n", stderr);
            return 1;
        }
    }
    for (pass = 0; pass < BENCH_PASSES; ++pass) {
        for (i = 0; i < CORPUS_SIZE; ++i) {
            const u8 *p = data + 16 + (i & 1023U);
            volatile u8 out = 0;
            long diff = guarded_negative_add_delta(p, choices[i], &out);
            checksum += (unsigned long long)(diff + 7) * 257U + out;
        }
    }
    printf("ptr_delta_phi_negative_add_bench: %llu\n", checksum);
    return 0;
}
