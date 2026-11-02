/* AArch64 direct nested-function static-chain support.  Address-taken
 * trampolines and non-local goto are separate features; this locks the common
 * direct-call path that fixes several GCC torture failures.
 */
extern void abort(void);
__attribute__((noinline)) int outer(int seed) {
    int captured = seed;
    int add(int x) { return captured + x; }
    return add(5);
}
int main(void) {
    if (outer(37) != 42) abort();
    return 0;
}
