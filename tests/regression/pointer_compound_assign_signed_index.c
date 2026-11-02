/* levkropp audit: pointer +=/-= must widen the integer index to pointer
 * width before scaling. A negative i32 index must sign-extend, not become a
 * huge positive 64-bit offset.
 */
extern void abort(void);
struct S { long a, b, c; };
__attribute__((noinline)) struct S *step(struct S *p, int i) {
    p += i;
    return p;
}
__attribute__((noinline)) struct S *step_back(struct S *p, int i) {
    p -= i;
    return p;
}
int main(void) {
    struct S a[4];
    if (step(&a[2], -1) != &a[1]) abort();
    if (step_back(&a[1], -1) != &a[2]) abort();
    return 0;
}
