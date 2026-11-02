#include <stdint.h>
#include <stdio.h>

extern uint64_t upper32_partial_word(void);
extern uint64_t upper32_divide(uint64_t);
extern uint64_t upper32_cmpxchg_implicit(void);
extern uint64_t upper32_cmpxchg_explicit(void);
extern uint64_t upper32_cmpxchg_locked(uint64_t *);
extern uint64_t upper32_negative_mask(void);

int main(void) {
    uint64_t cell = UINT64_C(0x100000006);
    uint64_t got[] = {
        upper32_partial_word(), upper32_divide(127),
        upper32_cmpxchg_implicit(), upper32_cmpxchg_explicit(),
        upper32_cmpxchg_locked(&cell), upper32_negative_mask(), cell,
    };
    const uint64_t want[] = {128, 63, 6, 6, 6, 0, UINT64_C(0x100000006)};
    unsigned failures = 0;
    for (unsigned i = 0; i < sizeof(got) / sizeof(got[0]); ++i) {
        if (got[i] != want[i]) {
            printf("case %u: got %#llx, expected %#llx\n", i,
                   (unsigned long long)got[i], (unsigned long long)want[i]);
            ++failures;
        }
    }
    printf("upper32 self-moves: %u failures\n", failures);
    return failures != 0;
}
