/* AArch64 has no byte compare: comparing an I8 IR value resident in a wider
 * packed register must sign/zero-extend the low byte first. Reduced from
 * gcc.c-torture/execute/20171008-1.c.
 */
extern void abort(void);
struct S { char c1, c2, c3, c4; } __attribute__((aligned(4)));
__attribute__((noinline)) struct S make(void) {
    struct S s;
    s.c1 = 0;
    s.c2 = 7;
    return s;
}
int main(void) {
    struct S s = make();
    if (s.c1 != 0) abort();
    return 0;
}
