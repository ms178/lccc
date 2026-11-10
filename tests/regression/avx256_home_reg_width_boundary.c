/* A 256-bit "home contents are in a YMM register" claim must never be used to
 * satisfy a narrower read.
 *
 * Keeping a loop-invariant vector constant in a register requires the backend
 * to remember, per home, which register currently holds that home's contents.
 * Such a claim is only valid for a read of the FULL width that was stored: a
 * 32-byte store leaves all 32 bytes of the register meaningful, but a 16-byte
 * store leaves the upper half untouched, so a 256-bit read could not be served
 * from it.  The claim is therefore recorded only for "ymmN" names written by
 * 32-byte stores, and the home's stack slot is still written every time.
 *
 * This fixture is the guard against getting either half of that wrong:
 *
 *   A. an invariant read only at 256 bits, many times (the claim fires);
 *   B. the SAME invariant additionally read through memory at 128 bits, at both
 *      halves, interleaved between the 256-bit reads -- so if the slot store
 *      were ever elided or deferred past a narrower read, the halves would
 *      come back as garbage;
 *   C. an invariant written once and then OVERWRITTEN with a different value,
 *      which must retire the claim so the new value is what later reads see.
 *
 * Every result is checked against a scalar reference. */
#include <string.h>
typedef float vf8 __attribute__((vector_size(32)));
typedef float vf4 __attribute__((vector_size(16)));

static float x86_min(float a, float b) {
    if (a != a || b != b) return a;
    if (a == b) return a;
    return b < a ? b : a;
}

/* A: invariant used only through 256-bit ops. */
__attribute__((noinline)) void wide_only(vf8 *out, const vf8 *in) {
    vf8 hi = {1, 2, 3, 4, 5, 6, 7, 8};
    out[0] = __builtin_ia32_minps256(in[0], hi);
    out[1] = __builtin_ia32_minps256(in[1], hi);
    out[2] = __builtin_ia32_minps256(in[2], hi);
    out[3] = __builtin_ia32_minps256(in[3], hi);
}

/* B: the same invariant, but each half is also read out through memory. */
__attribute__((noinline)) void halves_between(vf8 *out, vf4 *halves, const vf8 *in) {
    vf8 hi = {1, 2, 3, 4, 5, 6, 7, 8};
    out[0] = __builtin_ia32_minps256(in[0], hi);
    memcpy(&halves[0], (const char *)&hi, 16);
    out[1] = __builtin_ia32_minps256(in[1], hi);
    memcpy(&halves[1], (const char *)&hi + 16, 16);
    out[2] = __builtin_ia32_minps256(in[2], hi);
}

/* C: the invariant is rewritten, so any earlier claim must be retired. */
__attribute__((noinline)) void rewritten(vf8 *out, const vf8 *in) {
    vf8 bound = {1, 1, 1, 1, 1, 1, 1, 1};
    out[0] = __builtin_ia32_minps256(in[0], bound);
    bound = (vf8){9, 9, 9, 9, 9, 9, 9, 9};
    out[1] = __builtin_ia32_minps256(in[1], bound);
}

int main(void) {
    const float bound[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    vf8 in[4], out[4];
    for (int i = 0; i < 4; ++i) {
        union { vf8 v; float f[8]; } x;
        for (int j = 0; j < 8; ++j) x.f[j] = (float)(20 + i * 8 + j);
        in[i] = x.v;
    }

    wide_only(out, in);
    for (int i = 0; i < 4; ++i) {
        union { vf8 v; float f[8]; } g;
        g.v = out[i];
        for (int j = 0; j < 8; ++j) {
            float want = x86_min(bound[j], (float)(20 + i * 8 + j));
            if (g.f[j] != want) return 1 + i * 8 + j;
        }
    }

    vf4 halves[2];
    halves_between(out, halves, in);
    {   /* The halves are the raw invariant, read from memory, not clamped. */
        union { vf4 v; float f[4]; } h0, h1;
        h0.v = halves[0];
        h1.v = halves[1];
        for (int j = 0; j < 4; ++j) {
            if (h0.f[j] != bound[j]) return 100 + j;
            if (h1.f[j] != bound[j + 4]) return 200 + j;
        }
    }
    for (int i = 0; i < 3; ++i) {
        union { vf8 v; float f[8]; } g;
        g.v = out[i];
        for (int j = 0; j < 8; ++j) {
            float want = x86_min(bound[j], (float)(20 + i * 8 + j));
            if (g.f[j] != want) return 300 + i * 8 + j;
        }
    }

    rewritten(out, in);
    for (int i = 0; i < 2; ++i) {
        union { vf8 v; float f[8]; } g;
        g.v = out[i];
        float b = i == 0 ? 1.0f : 9.0f;
        for (int j = 0; j < 8; ++j) {
            float want = x86_min(b, (float)(20 + i * 8 + j));
            if (g.f[j] != want) return 400 + i * 8 + j;
        }
    }
    return 0;
}
