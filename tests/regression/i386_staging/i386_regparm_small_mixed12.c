/* A packed { unsigned long long; float } is twelve bytes and belongs to two classes
 * at once: eight of INTEGER and four of SSE.
 *
 * The two halves of the classification are the whole point of this fixture.  On
 * -m64 the object does not fit the two-integer prefix, so the integer half goes in
 * %rdi and the float half in %xmm0 -- as a *4-byte* move; the callee side of that is
 * `movq %xmm0, 16(%rsp)` in lccc's own output.  On -m32 -mregparm=3 there are exactly
 * three integer registers, and gcc stages the whole object as three `movl`s with no
 * SSE move at all (measured: `movl $1, %eax; movl $1, %edx; movl $1077936128, %ecx`).
 * Neither answer may be reached by grabbing 16 bytes: a `movaps`/`movups` over a
 * 12-byte argument reads four bytes the caller does not own, and it is exactly the
 * move an exit status cannot see, because the callee never looks at the extra bytes.
 *
 * The value is therefore not a pinned number.  gcc answers 40 for the naive version
 * of this program at -O2 and 98 at -O0 (-m32 -mregparm=3), so "agree with the
 * oracle" would be pinning one of two disagreeing oracles -- true divergence about
 * how a packed 12-byte struct is classified at -O0, and not a law anyone should
 * latch to.  What *is* a law: passing an object by value cannot change the object.
 * `main` computes the answer twice, once through the call and once from the members
 * directly, and the surrounding bytes are poisoned to 0xAA first, so a staging
 * sequence that reads wider than the subobject it was given has to show up as a
 * non-zero exit -- in any compiler, at any tier.  0 means the round trip preserved
 * the object; 1 means it did not, and that is what the gate asserts.
 */
struct P {
    unsigned long long w;
    float f;
} __attribute__((packed));

/* Volatile, so neither call can be folded away: the gate's assembly arms need the
 * staging sequence to actually be emitted. */
static volatile unsigned long long seed_w;
static volatile float seed_f;

__attribute__((noinline)) static unsigned long long use(struct P p)
{
    return p.w + (unsigned long long)(int)p.f;
}

int main(void)
{
    union {
        struct P p;
        unsigned char raw[16];
    } u;
    unsigned long long direct;

    seed_w = 0x100000001ULL;
    seed_f = 3.0f;
    for (unsigned i = 0; i < sizeof u.raw; i++)
        u.raw[i] = 0xAA;
    u.p.w = seed_w;
    u.p.f = seed_f;
    direct = u.p.w + (unsigned long long)(int)u.p.f;
    if (use(u.p) != direct)
        return 1;
    /* Twice through the ABI: a read of uninitialised surroundings is not stable
     * across two calls that pass the same bytes, and this catches the case where
     * the first comparison agreed by luck. */
    return use(u.p) != direct;
}
