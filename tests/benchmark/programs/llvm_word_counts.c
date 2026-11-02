/* Synthetic call bundle: not a production application speedup. */
#define WORD_COUNTS_BENCH
#include "../../regression/llvm_word_counts.c"
#ifndef ROUNDS
#define ROUNDS 8000000u
#endif
int main(void) {
    uint32_t state=1;uint64_t sum=0;
    for(unsigned i=0;i<ROUNDS;i++) {
        state=state*1664525u+1013904223u;
        unsigned x=state,y=state^(state>>13);
        sum+=count_min_trailing(x,y)+count_min_leading(x,y);
        sum+=count_cap_trailing(x)+count_cap_leading(x);
    }
    printf("%016llx\n",(unsigned long long)sum);return 0;
}
