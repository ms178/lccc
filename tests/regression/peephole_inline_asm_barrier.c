/* Regression: the x86 text peephole kept slot<->register and copy mappings
 * alive across `#APP`/`#NO_APP` inline-asm regions.  An asm output
 * ("=&r") redefining a register that a prior store had been recorded from
 * let global_store_forwarding rewrite the reload of the OLD value as a
 * register copy of the NEW one (framecall stress family, seed 0):
 *
 *     movq %rcx, 32(%rsp)      ; p2 (asm #2 output) spilled
 *     #APP ... (writes %rcx) #NO_APP   ; asm #3 output, different value
 *     movq 32(%rsp), %r11  ==>  movq %rcx, %r11      (WRONG)
 *
 * Fix: InlineAsm is a barrier for every state-tracking pass
 * (types.rs is_barrier, store_forwarding, copy_propagation, writes_family).
 * The shapes below exercise store forwarding, copy propagation and
 * callee-saved clobbers through asm with outputs, in-place operands ("+r"),
 * memory clobbers and register clobbers.  Expected values are computed by
 * plain C on the same inputs in a separate translation-unit-free way (the
 * reference arithmetic uses only volatile inputs and no asm). */
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static uint64_t mix(uint64_t x) { return (x * 0x9E3779B97F4A7C15ull) ^ (x >> 29); }

/* Shape 1: the exact stress-lab reproducer (constant args, inlined). */
static inline uint64_t fc1(uint64_t p0, uint64_t p1, uint64_t p2, uint64_t p3, uint64_t p4) {
    uint64_t r0 = mix(p4 ^ p3);
    { uint64_t t; __asm__ volatile("movq %1, %0\n\txorq %%r14, %%r14\n\taddq $3, %0" : "=&r"(t) : "r"(p2) : "r14", "cc"); p2 = t; }
    uint64_t r1 = mix(p1 ^ p4);
    { uint64_t t; __asm__ volatile("movq %1, %0\n\txorq %%r13, %%r13\n\taddq $3, %0" : "=&r"(t) : "r"(p2) : "r13", "cc"); p2 = t; }
    uint64_t r2 = mix(p3 ^ p1);
    { uint64_t t; __asm__ volatile("movq %1, %0\n\txorq %%r15, %%r15\n\taddq $3, %0" : "=&r"(t) : "r"(r0) : "r15", "cc"); r0 = t; }
    uint64_t r3 = mix(p0 ^ p2);
    { uint64_t t; __asm__ volatile("movq %1, %0\n\txorq %%r15, %%r15\n\taddq $3, %0" : "=&r"(t) : "r"(r0) : "r15", "cc"); r0 = t; }
    return (r3 * 1u) ^ (r1 * 2u) ^ (r2 * 3u) ^ (p4 * 4u) ^ (p2 * 5u) ^ (r0 * 6u) ^ (p3 * 7u) ^ (p0 * 8u) ^ (p1 * 9u);
}
static uint64_t fc1_ref(uint64_t p0, uint64_t p1, uint64_t p2, uint64_t p3, uint64_t p4) {
    uint64_t r0 = mix(p4 ^ p3); p2 += 3;
    uint64_t r1 = mix(p1 ^ p4); p2 += 3;
    uint64_t r2 = mix(p3 ^ p1); r0 += 3;
    uint64_t r3 = mix(p0 ^ p2); r0 += 3;
    return (r3 * 1u) ^ (r1 * 2u) ^ (r2 * 3u) ^ (p4 * 4u) ^ (p2 * 5u) ^ (r0 * 6u) ^ (p3 * 7u) ^ (p0 * 8u) ^ (p1 * 9u);
}

/* Shape 2: copy propagation across an in-place ("+r") asm operand. */
__attribute__((noinline)) uint64_t inplace(uint64_t a, uint64_t b) {
    uint64_t c = a;                       /* copy c <- a */
    __asm__ volatile("addq %1, %0" : "+r"(c) : "r"(b) : "cc");  /* c redefined in place */
    return c ^ (a << 1);                  /* must read the NEW c, not a */
}

/* Shape 3: asm writes a value through memory ("memory" clobber) that a
 * prior store had left in a register mapping. */
__attribute__((noinline)) uint64_t memclobber(uint64_t a, uint64_t b) {
    uint64_t slot = a;
    uint64_t *p = &slot;
    __asm__ volatile("movq %1, (%0)" : : "r"(p), "r"(b) : "memory");
    return slot + a;                      /* slot is now b */
}

/* Shape 4: many live values + asm clobbering callee-saved registers. */
__attribute__((noinline)) uint64_t clobbers(uint64_t a, uint64_t b, uint64_t c, uint64_t d, uint64_t e, uint64_t f) {
    uint64_t x = mix(a), y = mix(b), z = mix(c), w = mix(d), v = mix(e), u = mix(f);
    __asm__ volatile("xorq %%rbx, %%rbx\n\txorq %%r12, %%r12\n\txorq %%r13, %%r13\n\txorq %%r14, %%r14\n\txorq %%r15, %%r15"
                     : : : "rbx", "r12", "r13", "r14", "r15", "cc");
    return x ^ (y << 1) ^ (z << 2) ^ (w << 3) ^ (v << 4) ^ (u << 5) ^ a ^ b ^ c ^ d ^ e ^ f;
}
static uint64_t clobbers_ref(uint64_t a, uint64_t b, uint64_t c, uint64_t d, uint64_t e, uint64_t f) {
    uint64_t x = mix(a), y = mix(b), z = mix(c), w = mix(d), v = mix(e), u = mix(f);
    return x ^ (y << 1) ^ (z << 2) ^ (w << 3) ^ (v << 4) ^ (u << 5) ^ a ^ b ^ c ^ d ^ e ^ f;
}

static volatile uint64_t A = 2664053391046770798ull, B = 2444141519646934967ull, C = 14274212106794373783ull,
                         D = 4968186882940395168ull, E = 9980036131371284697ull, F = 0x1234567890abcdefull;

int main(void) {
    int fails = 0;
#define CHECK(name, got, want) do { unsigned long long g = (got), w = (want); \
        if (g != w) { fails++; printf("FAIL %s got %llu want %llu\n", name, g, w); } } while (0)
    CHECK("fc1-const", fc1(2664053391046770798ull, 2444141519646934967ull, 14274212106794373783ull,
                           4968186882940395168ull, 9980036131371284697ull), 733194002982536891ull);
    CHECK("fc1-rt", fc1(A, B, C, D, E), fc1_ref(A, B, C, D, E));
    CHECK("inplace", inplace(A, B), (A + B) ^ (A << 1));
    CHECK("memclobber", memclobber(A, B), A + B);
    CHECK("clobbers", clobbers(A, B, C, D, E, F), clobbers_ref(A, B, C, D, E, F));
    if (fails == 0) puts("ALL OK");
    return fails;
}
