/* Push-adjusted rsp-relative addressing must not be deduplicated.
 *
 * eliminate_redundant_leaq used to treat `leaq X(%rsp), %rax` as identical
 * across pushes that decrement %rsp. Around an sret call with a by-value
 * struct argument, the pre-push argument address and post-push return-buffer
 * address can have the same textual displacement while naming different stack
 * locations. The callee then writes the result over the argument.
 */
struct W { double a, b, c; unsigned k : 12, j : 13, i : 7; };

struct W retme(struct W x) { return x; }

__attribute__((noinline)) unsigned roundtrip(struct W y) {
    y.k += 5;
    y = retme(y);
    return y.k + y.j + y.i;
}

int main(void) {
    struct W w;
    __builtin_memset(&w, 0, sizeof w);
    w.k = 100;
    w.j = 200;
    w.i = 3;
    return roundtrip(w) == 308 ? 0 : 1;
}
