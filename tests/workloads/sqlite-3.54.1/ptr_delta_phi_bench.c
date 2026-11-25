/*
 * Standalone, cache-resident VM A/B screening for the actual SQLite 3.54.1
 * sqlite3GetVarint control-flow shape, not the older separately-returning
 * decoder in tests/benchmark/programs/sqlite_varint.c. Public domain.
 *
 * Keep the decoder and the 90,000-case independent oracle in ONE source:
 * a benchmark is only admissible when its executable runs that oracle first.
 * Timings include process startup, so use the repository's interleaved
 * perf_ab.py runner (11+ pairs), not a single unpaired wall-clock reading.
 */
#define main sqlite_varint_selfcheck
#include "ptr_delta_phi.c"
#undef main

#define CORPUS_SIZE 4096U
#ifndef BENCH_PASSES
#define BENCH_PASSES 8000U
#endif

static u8 bench_corpus[CORPUS_SIZE][9];

int main(void){
  unsigned i, j, pass;
  u64 total_value = 0;
  u64 total_length = 0;
  if(sqlite_varint_selfcheck() != 0) return 1;
  for(i=0; i<CORPUS_SIZE; ++i){
    u64 actual, reference;
    unsigned n = 1U + i%9U;
    u8 len, expected;
    for(j=0; j<n-1U; ++j){
      bench_corpus[i][j] = (next_byte() & 0x7fU) | 0x80U;
    }
    bench_corpus[i][n-1U] = next_byte() & (n<9U ? 0x7fU : 0xffU);
    len = sqlite_varint_delta(bench_corpus[i], &actual);
    expected = reference_decode(bench_corpus[i], &reference);
    if((unsigned)len!=n || (unsigned)expected!=n || actual!=reference) return 2;
  }
  for(pass=0; pass<BENCH_PASSES; ++pass){
    for(i=0; i<CORPUS_SIZE; ++i){
      u64 result;
      total_length += sqlite_varint_delta(bench_corpus[i], &result);
      total_value += result;
    }
  }
  printf("ptr_delta_phi_bench: %llu %llu\n",
         (unsigned long long)total_length, (unsigned long long)total_value);
  return 0;
}
