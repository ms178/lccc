/* FP copy-web leader eligibility (x86-64 XMM scan).
 *
 * `t = h2(...)` is an F32 call result that stays live across the later
 * `leaf()` call and is copied into the phi web of `t` on the else arm.  The
 * union-find root of that web was the call-spanning `t`, which the scan
 * filter excludes; the merge still seeded the scan with it as the leader,
 * using the envelope of the eligible members (which does not cover the
 * `leaf()` call).  `t` was homed in a caller-saved XMM, the call clobbered
 * it, and the peephole correctly deleted the now-dead result move: the else
 * arm then read a stale register (wrong output at -O1 and above).  The
 * leader must be a scan-eligible member; the root keeps its own home.
 */
#include <stdio.h>
#include <string.h>

static unsigned bits(float f)
{
    unsigned u;
    memcpy(&u, &f, 4);
    return u;
}

static float fromb(unsigned u)
{
    float f;
    memcpy(&f, &u, 4);
    return f;
}

__attribute__((noinline)) float h2(float a, float b, float c, float d)
{
    return (a + b) * (c - d);
}

__attribute__((noinline)) unsigned leaf(float a, float b, unsigned m)
{
    float t = a * b;
    unsigned u = bits(t) ^ m;
    float s = fromb(u & 0x3f7fffffu);
    return bits(s + a) + (a < b) + (t >= s) * 2u;
}

__attribute__((noinline)) unsigned f(const float *in, const unsigned *mi, int n)
{
    float v1 = in[1] * 1.146f + 0.333f;
    float v2 = in[2] * -2.766f + -0.062f;
    float v4 = in[4] * 0.488f + -0.336f;
    float v7 = in[7] * -0.518f + 0.169f;
    float v8 = in[0] * -1.189f + 1.947f;
    float v12 = in[4] * -1.266f + 0.174f;
    float t = in[5] * 1.437f + -0.711f;
    float v15 = in[7] * -0.004f + 0.423f;
    unsigned acc = 0;
    for (int it = 0; it < n; it++) {
        t = h2(v8, v2, t, (float)it);
        acc += leaf(v12, v7, mi[it & 3]);
        if (v12 < v4)
            t = v12 - v4;
        else
            acc ^= bits(t);
        t = h2(v1, v12, t, (float)it);
        v12 = fromb((bits(v15) & 0x807fffffu) | 0x3f000000u);
    }
    return acc;
}

int main(void)
{
    float in[8];
    const unsigned mi[4] = {0x1234u, 0xffu, 0x80000000u, 7u};
    for (int i = 0; i < 8; i++)
        in[i] = (float)(i * 7 % 5) - 1.5f + i * 0.125f;
    printf("%08x\n", f(in, mi, 37));
    return 0;
}
