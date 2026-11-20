/* On i686 %eax is both the accumulator and a register the allocator hands
 * out as a home. A value computed straight into an %eax home overwrites the
 * register the accumulator cache describes, so the cache has to follow.
 *
 * In `mix` below the product `acc * K` is stored to its home and stays
 * parked in %eax; half of the sum is then computed directly into ITS home,
 * which is %eax as well. The final add asked for the product "still in
 * %eax", got the half sum, and added the second term to itself:
 *
 *     movl %ebp, %eax ; imull %ecx, %eax ; movl %eax, %edx   product, parked
 *     movl %ebp, %eax ; addl $42007, %eax                    half sum -> %eax
 *     movl %eax, %ebp ; addl 16(%esp), %ebp                  sum
 *     addl %ebp, %eax                                        "product" + sum
 *
 * `walk` is the function a random scope-lifetime program was reduced to; its
 * unrelated control flow is what steers the allocator into that assignment,
 * so it is kept whole. `mix` is the arithmetic on its own under the same
 * register pressure. Both are checked against straight-line references that
 * keep every intermediate in memory, and the output is compared with the
 * reference compiler's. */
#include <stdio.h>

typedef unsigned int u32;
typedef unsigned char u8;

static u32 acc;
#define MIX(v) (acc = acc * 2654435761u + (u32)(v))

__attribute__((noinline)) static void walk(const u8 *in)
{
    u8 o15[8];
    int n20 = 0;
    for (int i4 = 0; i4 < 2; i4++) {
        switch (in[12] % 5) {
        case 1: {
            if (in[5] & 4) {
                u8 o19[8];
                u32 o21[4];
                for (int i = 0; i < 4; i++)
                    o21[i] = (u32)(i * 56u + 41839u + acc);
                o19[acc % 8] += (u8)acc;
                o19[3] ^= 3;
                MIX(o21[3] + o21[1]);
            } else {
                if (in[1] & 8) {
                } else {
                B17:;
                    if (n20++ < 2 && (in[12] & 1)) {
                        goto B17;
                    }
                }
            }
            switch (in[5] % 4) {
            case 0: {
                for (int i = 0; i < 8; i++)
                    o15[i] = (u8)(i * 76u + 38834u + acc);
                MIX(o15[0]);
            } break;
            }
        } /* falls through */
        }
    }
}

/* What `walk` computes for the input below, one step per statement and every
 * intermediate volatile so nothing is parked anywhere. */
__attribute__((noinline)) static u32 walk_reference(u32 start)
{
    volatile u32 a = start;
    for (int round = 0; round < 2; round++) {
        volatile u32 third = 3u * 56u + 41839u + a;
        volatile u32 first = 1u * 56u + 41839u + a;
        volatile u32 product = a * 2654435761u;
        volatile u32 sum = third + first;
        a = product + sum;
        volatile u32 byte = (u8)(0u * 76u + 38834u + a);
        volatile u32 product2 = a * 2654435761u;
        a = product2 + byte;
    }
    return a;
}

/* The arithmetic alone: six values live across the statement keep the
 * callee-saved registers busy, so the temporaries land in %eax/%edx. */
__attribute__((noinline)) static u32 mix(u32 a, u32 b, u32 c, u32 d, const u32 *v)
{
    u32 r = a;
    for (int i = 0; i < 4; i++) {
        u32 x = v[i] + r;
        u32 y = v[(i + 1) & 3] + r;
        r = r * 2654435761u + ((x + 42007u) + (y + 41895u));
        r ^= b + (u32)i;
        b += c ^ d;
        c += d + x;
        d ^= y;
    }
    return r + b + c + d;
}

__attribute__((noinline)) static u32 mix_reference(u32 a, u32 b, u32 c, u32 d, const u32 *v)
{
    volatile u32 r = a, vb = b, vc = c, vd = d;
    for (int i = 0; i < 4; i++) {
        volatile u32 x = v[i] + r;
        volatile u32 y = v[(i + 1) & 3] + r;
        volatile u32 product = r * 2654435761u;
        volatile u32 left = x + 42007u;
        volatile u32 right = y + 41895u;
        volatile u32 sum = left + right;
        r = product + sum;
        r ^= vb + (u32)i;
        vb += vc ^ vd;
        vc += vd + x;
        vd ^= y;
    }
    return r + vb + vc + vd;
}

int main(void)
{
    static const u8 input[16] = {3, 2, 5, 5, 7, 4, 4, 1, 4, 3, 7, 0, 1, 3, 3, 6};
    static const u32 table[4] = {0x9e3779b9u, 0x7f4a7c15u, 0xf39cc060u, 0x5cedc834u};
    int bad = 0;

    for (u32 start = 0; start < 5; start++) {
        acc = start * 0x01000193u;
        u32 want = walk_reference(acc);
        walk(input);
        printf("walk %u: %u%s\n", start, acc, acc == want ? "" : " WRONG");
        bad += acc != want;
    }
    for (u32 seed = 1; seed < 6; seed++) {
        u32 got = mix(seed, seed * 3u, seed ^ 0x55u, seed + 77u, table);
        u32 want = mix_reference(seed, seed * 3u, seed ^ 0x55u, seed + 77u, table);
        printf("mix %u: %u%s\n", seed, got, got == want ? "" : " WRONG");
        bad += got != want;
    }
    printf("%s\n", bad ? "FAILED" : "ok");
    return bad != 0;
}
