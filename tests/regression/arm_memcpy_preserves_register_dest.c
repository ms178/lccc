/* AArch64 memcpy lowering resolves dest into x9, then source into x9/x10.
 * When dest was a register-allocated pointer, source resolution used to clobber
 * the dest and copy the source to itself. Reduced from the _Complex long double
 * half of gcc.c-torture/execute/20050121-1.c.
 */
extern void abort(void);
struct Q { unsigned long a, b; };
__attribute__((noinline)) void copy_to_arg(struct Q *dst, struct Q *src) {
    *dst = *src;
}
int main(void) {
    struct Q dst = {1, 2}, src = {3, 4};
    copy_to_arg(&dst, &src);
    if (dst.a != 3 || dst.b != 4) abort();
    return 0;
}
