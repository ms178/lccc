/* By-value arguments read out of over-aligned locals.
 *
 * AArch64 addresses an alloca aligned beyond 16 bytes as
 * `(sp + offset + align - 1) & -align`.  The helper staged both constants
 * in x17, which is also the base register the call lowering uses to copy a
 * by-value aggregate into x1/x2 and the address register of binary128
 * loads: `add x17, sp, #32; movz x17, #31; ...; and x17, x17, x17` left
 * -32 in x17 and the copy faulted (gcc.c-torture execute/pr92904.c at -O1,
 * a union with a 32-byte-aligned member passed to a variadic function).
 * The bias now folds into the frame offset and the mask is a logical
 * immediate, so no scratch register is involved.
 *
 * Self-checking: exit status 0 is success; abort() otherwise. */
extern void abort(void);

struct pair { long long a, b; };
struct __attribute__((aligned(32))) quad { double a, b, c, d; };
union holder {
    struct pair p;
    struct quad q;
};

__attribute__((noinline)) long long take_pair(int bias, struct pair p) { return p.a - p.b + bias; }
__attribute__((noinline)) long long take_pair_late(int a, int b, int c, struct pair p) { return p.a ^ p.b ^ a ^ b ^ c; }
__attribute__((noinline)) double take_quad(struct quad q) { return q.a + q.b + q.c + q.d; }

int main(void)
{
    union holder u, v;
    u.p.a = 0x5555555555555555ll;
    u.p.b = 0x1111111111111111ll;
    if (take_pair(1, u.p) != 0x4444444444444445ll)
        abort();
    if (take_pair_late(1, 2, 4, u.p) != (0x4444444444444444ll ^ 7))
        abort();
    v.q.a = 1.0;
    v.q.b = 2.0;
    v.q.c = 4.0;
    v.q.d = 8.0;
    if (take_quad(v.q) != 15.0)
        abort();
    if (((unsigned long)&u & 31) != 0 || ((unsigned long)&v & 31) != 0)
        abort();
    return 0;
}
