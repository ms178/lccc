/* Backedge PRE correctness: carry x*x from the previous latch to the next
 * iteration header for integer recurrences. The optimization must preserve
 * unsigned wraparound semantics.
 */
typedef unsigned long long u64;
extern void abort(void);
__attribute__((noinline)) u64 run(u64 n) {
    u64 x = 1, acc = 0;
    for (u64 i = 0; i < n; ++i) {
        u64 y = x * x;
        x += 3;
        u64 z = x * x;
        acc += (y ^ (z >> 17));
    }
    return acc ^ x;
}
int main(void) {
    if (run(1000) != 2998507822ULL) abort();
    return 0;
}
