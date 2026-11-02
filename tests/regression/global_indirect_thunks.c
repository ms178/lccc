/* Non-kernel ABI/security fixture. Global-cell folding must not bypass an
 * explicitly selected indirect-branch mitigation. Deliberately mutable external
 * cells prevent constant devirtualization; argument seven exercises stack setup. */
typedef long (*callback)(long, long, long, long, long, long, long);
extern callback slot;
extern struct callbacks { long padding; callback call; } table;
__attribute__((noinline)) long invoke_slot(long n) {
    return slot(n, 2, 3, 4, 5, 6, 7) + 1;
}
__attribute__((noinline)) long invoke_member(long n) {
    return table.call(n, 2, 3, 4, 5, 6, 7) + 2;
}
__attribute__((noinline)) long invoke_local(callback f, long n) {
    return f(n, 2, 3, 4, 5, 6, 7) + 3;
}
__attribute__((noinline)) long sum(long a, long b, long c, long d,
                                  long e, long f, long g) {
    return a + b + c + d + e + f + g;
}
callback slot = sum;
struct callbacks table = {0, sum};
__attribute__((noinline)) long replacement(long a, long b, long c, long d,
                                          long e, long f, long g) {
    return a - b - c - d - e - f - g;
}
__attribute__((noinline)) long invoke_snapshot(long n) {
    callback captured = slot;
    slot = replacement;
    return captured(n, 2, 3, 4, 5, 6, 7) + 4;
}
callback volatile volatile_slot = sum;
__attribute__((noinline)) long invoke_member_snapshot(long n) {
    callback captured = table.call;
    table.call = replacement;
    return captured(n, 2, 3, 4, 5, 6, 7) + 5;
}
__attribute__((noinline)) long invoke_volatile_snapshot(long n) {
    callback captured = volatile_slot;
    volatile_slot = replacement;
    return captured(n, 2, 3, 4, 5, 6, 7) + 6;
}
int main(void) {
    if (invoke_slot(1) != 29) return 1;
    if (invoke_member(1) != 30) return 2;
    if (invoke_local(sum, 1) != 31) return 3;
    if (invoke_snapshot(1) != 32) return 4;
    if (invoke_member_snapshot(1) != 33) return 5;
    if (invoke_volatile_snapshot(1) != 34) return 6;
    return 0;
}
