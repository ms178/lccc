/*
 * Read64-style unaligned load forwarding lock.
 *
 * The idiomatic unaligned scalar load is
 *     T v; __builtin_memcpy(&v, p, sizeof T); return v;
 * which lowers (after inlining) to `Alloca v; memcpy(&v,p,N); Load v`.  After
 * the 2026-09-09 change, constant-size __builtin_memcpy lowers to the native
 * `Instruction::Memcpy` so the SROA load-forwarding pass can collapse the
 * alloca+memcpy+load into a single unaligned load (GCC parity) instead of a
 * store-to-temp + reload.
 *
 * Every case here must still be SEMANTICALLY correct at -O0/-O1/-O2/-O3:
 *   - unaligned 64/32/16-bit reads through __builtin_memcpy;
 *   - the value is captured BEFORE a store that could alias the source, so the
 *     peephole fold_ptr_deref_through_stack must keep the slot read (this is
 *     also the fp_liveness_ptr_deref_alias_negative soundness probe);
 *   - __builtin_memmove (overlap) is NEVER rewritten to a no-overlap Memcpy.
 *
 * exit 0 = pass (tests/regression/runner convention).
 */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

typedef unsigned long long u64;
typedef unsigned int u32;
typedef unsigned short u16;

/* idiomatic unaligned loads */
static inline u64 rd64(const void *p) { u64 v; __builtin_memcpy(&v, p, 8); return v; }
static inline u32 rd32(const void *p) { u32 v; __builtin_memcpy(&v, p, 4); return v; }
static inline u16 rd16(const void *p) { u16 v; __builtin_memcpy(&v, p, 2); return v; }

/* The peephole soundness case: value captured before an aliasing store.
 * p and q alias (caller passes the same address), so the load of *p must not
 * be re-ordered past *q=0 and folded into a re-read of *p. */
__attribute__((noinline)) double g_alias(double *p, double *q) {
    u64 bits;
    __builtin_memcpy(&bits, p, 8);   /* bits = *p  (captured) */
    *q = 0.0;                        /* aliasing store      */
    double d;
    __builtin_memcpy(&d, &bits, 8);  /* d = bits            */
    return d + 1.0;
}

int main(void) {
    /* byte buffer with deliberately mis-aligned starts */
    unsigned char raw[128];
    for (int i = 0; i < 128; i++) raw[i] = (unsigned char)(i * 37 + 11);

    u64 ck = 0x1122334455667788ULL;
    for (int base = 1; base < 24; base++) {          /* base = mis-alignment */
        u64 x = rd64(raw + base);
        u32 y = rd32(raw + base);
        u16 z = rd16(raw + base);
        /* independently recompute */
        u64 xr = 0; for (int i = 0; i < 8; i++) xr |= ((u64)raw[base + i]) << (8 * i);
        u32 yr = 0; for (int i = 0; i < 4; i++) yr |= ((u32)raw[base + i]) << (8 * i);
        u16 zr = 0; for (int i = 0; i < 2; i++) zr |= ((u16)raw[base + i]) << (8 * i);
        if (x != xr || y != yr || z != zr) { printf("FAIL unaligned base=%d\n", base); return 1; }
        ck = ck * 1315423911ULL ^ x ^ (u64)y ^ (u64)z;
    }

    /* aliasing-store soundness: must be the captured 2.0, not the stored 0.0 */
    {
        double a, b;
        u64 pat = 0x4000000000000000ULL;             /* bit pattern of 2.0 */
        __builtin_memcpy(&a, &pat, 8);
        double r = g_alias(&a, &a);                  /* p==q==&a */
        /* a was set to 2.0, then *q=0 sets a=0.0, but g_alias captured bits
         * (2.0) before the store -> returns 2.0+1 = 3.0 */
        if (r != 3.0) { printf("FAIL aliasing-store: got %g\n", r); return 2; }
        (void)b;
    }

    /* __builtin_memmove overlap must stay memmove (never no-overlap Memcpy). */
    {
        unsigned char buf[32];
        for (int i = 0; i < 32; i++) buf[i] = (unsigned char)i;
        __builtin_memmove(buf + 4, buf, 28);         /* shift right */
        if (buf[4] != 0 || buf[5] != 1 || buf[31] != 27 || buf[0] != 0) {
            printf("FAIL memmove overlap\n"); return 3;
        }
    }

    printf("OK memcpy_unaligned_load_fwd %016llx\n", ck);
    return 0;
}
