/*
 * v9 regression: flag fusion through integer Casts (Cmp -> Cast -> branch).
 *
 * The v9 fusion lets a widened boolean feed a branch directly (cmp;jcc) with
 * no setcc/movzbl/test. Soundness rule: the cast destination must be used
 * exactly once, and a boolean used as DATA must still be materialized.
 * Both paths are exercised; differential vs GCC.
 */
#include <stdio.h>

static int as_branch(int x, int y) { if ((long long)(x < y)) return 1; return 2; }
static int via_u8(unsigned char c) { if ((int)(c >= 'a' && c <= 'z')) return 3; return 4; }
static int via_ll(int x) { return ((long long)(x < 100)) ? 7 : 8; }
static int via_ull(int x) { return ((unsigned long long)(x > -100)) ? 5 : 6; }

/* boolean used as DATA (returned): must materialize, not fuse. */
static long long as_data(int x, int y) { return (long long)(x < y); }
static unsigned char as_data2(unsigned char c) { return (unsigned char)(c >= 'a' && c <= 'z'); }

/* fused in a loop condition, then reused as data afterwards. */
static int loop_cond(int n) {
    int i, acc = 0;
    for (i = 0; (long long)(i < n); i++) acc += i;
    return acc;
}

int main(void) {
    int r = 0;
    r += as_branch(1, 2);
    r += as_branch(2, 1);
    r += via_u8('m');
    r += via_u8('9');
    r += via_ll(50);
    r += via_ll(150);
    r += via_ull(0);
    r += via_ull(-200);
    printf("%d %lld %u %d\n", r, as_data(3, 4) + as_data(4, 3),
           (unsigned)as_data2('q') + (unsigned)as_data2('1'),
           loop_cond(10));
    return 0;
}
