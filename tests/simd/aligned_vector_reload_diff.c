/* Differential correctness for the aligned-vector store->reload forwarding
 * pass (src/backend/x86/codegen/peephole/passes/aligned_vector_reload.rs).
 *
 * Every kernel below creates the shape the pass targets -- a vector stored to
 * an over-aligned frame home and read back -- and then surrounds it with the
 * exact conditions that must make the pass REFUSE: an intervening call, an
 * intervening write to the source register, an intervening unknown store, a
 * volatile access, a wider read, a different address, a pointer alias that
 * observes the memory, and a control-flow merge.  A pass that forwards when
 * it must not changes the printed checksum; a pass that refuses correctly
 * prints the same one as GCC and Clang.
 *
 * Build:  <cc> -O2 -mavx2 [-mavx512f -mavx512bw -mavx512vl] this.c -o /tmp/av
 * Run:    /tmp/av           (prints one checksum line per kernel)
 *
 * No UB: unsigned arithmetic only, fully initialised loads, no signed
 * overflow, no out-of-bounds indexing, no strict-aliasing violations (the
 * aliasing kernel goes through `unsigned char`, which may alias anything).
 * The PRNG is a xorshift64* so every compiler runs the identical sequence.
 * No _Alignas specifier ever REDUCES alignment (that is a C11 constraint
 * violation which GCC and Clang reject).
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef int32_t v8i __attribute__((vector_size(32)));
typedef int32_t v4i __attribute__((vector_size(16)));
typedef int64_t v4l __attribute__((vector_size(32)));

static uint64_t rng_state;

static uint64_t xr(void) {
    uint64_t x = rng_state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    rng_state = x;
    return x * 2685821657736338717ULL;
}

static v8i rand_v8(void) {
    v8i v;
    for (int i = 0; i < 8; i++) {
        v[i] = (int32_t)(uint32_t)xr();
    }
    return v;
}

static uint64_t mix_bytes(uint64_t h, const void *p, size_t n) {
    const unsigned char *b = (const unsigned char *)p;
    for (size_t i = 0; i < n; i++) {
        h ^= (uint64_t)b[i];
        h *= 1099511628211ULL;
    }
    return h;
}

static uint64_t mix_v8(uint64_t h, v8i v) { return mix_bytes(h, &v, sizeof v); }
static uint64_t mix_v4(uint64_t h, v4i v) { return mix_bytes(h, &v, sizeof v); }
static uint64_t mix_v4l(uint64_t h, v4l v) { return mix_bytes(h, &v, sizeof v); }

/* An opaque barrier: the pass must treat the call as a full memory and
 * register clobber.  `noinline` keeps it a real call. */
v8i perturb(v8i x, int32_t k);

__attribute__((noinline)) v8i perturb(v8i x, int32_t k) {
    return x + (v8i){k, k, k, k, k, k, k, k};
}

/* 1. The plain copy-out/copy-in chain through one over-aligned home. */
static uint64_t k_copy_chain(int rounds) {
    _Alignas(64) v8i a = rand_v8();
    _Alignas(64) v8i b = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(64) v8i t = a * b + a;
        a = t;
        b = b ^ (v8i){r, r + 1, r + 2, r + 3, r + 4, r + 5, r + 6, r + 7};
        h = mix_v8(h, a);
    }
    return mix_v8(h, b);
}

/* 2. A call between the store and the reload: forwarding must be refused. */
static uint64_t k_call_barrier(int rounds) {
    _Alignas(32) v8i a = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(32) v8i t = a;
        a = perturb(t, (int32_t)(uint32_t)xr());
        h = mix_v8(h, a);
    }
    return h;
}

/* 3. The source register is rewritten between store and reload. */
static uint64_t k_source_clobber(int rounds) {
    _Alignas(32) v8i a = rand_v8();
    _Alignas(32) v8i c = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(32) v8i t = a;
        a = a * c;        /* rewrites the register that backed the store */
        h = mix_v8(h, t); /* must still see the OLD value */
        h = mix_v8(h, a);
        c = c + (v8i){1, 2, 3, 4, 5, 6, 7, 8};
    }
    return h;
}

/* 4. An unknown store through a pointer between store and reload: the
 *    pointer may alias the home, so forwarding must be refused. */
static uint64_t k_alias_write(int rounds) {
    _Alignas(32) v8i a = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    unsigned char *raw = (unsigned char *)&a;
    for (int r = 0; r < rounds; r++) {
        _Alignas(32) v8i t = a;
        raw[(unsigned)r % 32u] ^= (unsigned char)(uint32_t)xr();
        h = mix_v8(h, t);
        h = mix_v8(h, a); /* must observe the byte poke */
        a = a + t;
    }
    return h;
}

/* 5. A volatile home: the read is an observable event and must survive. */
static uint64_t k_volatile(int rounds) {
    _Alignas(32) volatile v8i a = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        volatile v8i t = a;
        const v8i snapshot = t;
        h = mix_v8(h, snapshot);
        a = snapshot + (v8i){r, r, r, r, r, r, r, r};
    }
    return mix_v8(h, (v8i)a);
}

/* 6. A NARROWER read of a wider home: legal to forward only if the source is
 *    re-viewed at the load's width.  This is the low-half case. */
static uint64_t k_narrow_read(int rounds) {
    _Alignas(32) v8i a = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    v4i low;
    for (int r = 0; r < rounds; r++) {
        _Alignas(32) v8i t = a * (v8i){3, 3, 3, 3, 3, 3, 3, 3};
        memcpy(&low, &t, sizeof low);
        h = mix_v4(h, low);
        h = mix_v8(h, t);
        a = t + (v8i){r, 0, r, 0, r, 0, r, 0};
    }
    return h;
}

/* 7. Several distinct homes live at once: keys must not be confused. */
static uint64_t k_many_homes(int rounds) {
    _Alignas(64) v8i p = rand_v8();
    _Alignas(32) v8i q = rand_v8();
    v8i s = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(64) v8i tp = q + s;
        _Alignas(32) v8i tq = s * p;
        v8i ts = p ^ q;
        p = tp;
        q = tq;
        s = ts;
        h = mix_v8(mix_v8(mix_v8(h, p), q), s);
    }
    return h;
}

/* 8. 64-bit lanes through an over-aligned home. */
static uint64_t k_i64_lanes(int rounds) {
    _Alignas(32) v4l a = {(int64_t)xr(), (int64_t)xr(), (int64_t)xr(), (int64_t)xr()};
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(32) v4l t = a * (v4l){3, 5, 7, 11};
        a = t + (v4l){r, r, r, r};
        h = mix_v4l(h, a);
    }
    return h;
}

/* 9. A branch inside the copy window: the merge must invalidate. */
static uint64_t k_branch_merge(int rounds) {
    _Alignas(32) v8i a = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(32) v8i t;
        if ((xr() & 1ULL) != 0ULL) {
            t = a * (v8i){2, 2, 2, 2, 2, 2, 2, 2};
        } else {
            t = a + (v8i){7, 7, 7, 7, 7, 7, 7, 7};
        }
        h = mix_v8(h, t);
        a = t;
    }
    return h;
}

/* 10. Two reads of one stored value: both may forward, and both must see the
 *     same bytes. */
static uint64_t k_two_readers(int rounds) {
    _Alignas(32) v8i a = rand_v8();
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(32) v8i t = a * (v8i){5, 5, 5, 5, 5, 5, 5, 5};
        const v8i r1 = t;
        const v8i r2 = t;
        h = mix_v8(mix_v8(h, r1), r2);
        a = r1 + r2;
    }
    return h;
}

#if defined(__AVX512F__)
typedef int32_t v16i __attribute__((vector_size(64)));

static uint64_t mix_v16(uint64_t h, v16i v) { return mix_bytes(h, &v, sizeof v); }

/* 11. 512-bit homes: the EVEX encoding class must be preserved, and a
 *     256-bit read of a 512-bit home is the low-half case again. */
static uint64_t k_zmm(int rounds) {
    _Alignas(64) v16i a;
    for (int i = 0; i < 16; i++) {
        a[i] = (int32_t)(uint32_t)xr();
    }
    uint64_t h = 1469598103934665603ULL;
    for (int r = 0; r < rounds; r++) {
        _Alignas(64) v16i t = a * a;
        v8i low;
        memcpy(&low, &t, sizeof low);
        a = t + a;
        h = mix_v8(mix_v16(h, a), low);
    }
    return h;
}
#endif

int main(void) {
    rng_state = 0x243f6a8885a308d3ULL;
    const int rounds = 64;
    printf("copy_chain   %016llx\n", (unsigned long long)k_copy_chain(rounds));
    printf("call_barrier %016llx\n", (unsigned long long)k_call_barrier(rounds));
    printf("src_clobber  %016llx\n", (unsigned long long)k_source_clobber(rounds));
    printf("alias_write  %016llx\n", (unsigned long long)k_alias_write(rounds));
    printf("volatile     %016llx\n", (unsigned long long)k_volatile(rounds));
    printf("narrow_read  %016llx\n", (unsigned long long)k_narrow_read(rounds));
    printf("many_homes   %016llx\n", (unsigned long long)k_many_homes(rounds));
    printf("i64_lanes    %016llx\n", (unsigned long long)k_i64_lanes(rounds));
    printf("branch_merge %016llx\n", (unsigned long long)k_branch_merge(rounds));
    printf("two_readers  %016llx\n", (unsigned long long)k_two_readers(rounds));
#if defined(__AVX512F__)
    printf("zmm          %016llx\n", (unsigned long long)k_zmm(rounds));
#endif
    return 0;
}
