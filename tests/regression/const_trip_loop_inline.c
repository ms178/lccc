/*
 * Constant-trip loop-helper inlining: run-time semantics.
 *
 * `mix_rounds` has the SipHash `sip_round` shape: a loop whose exit compares
 * a parameter, and four state words updated through a pointer. Its body is
 * about 137 IR instructions, so the general static-loop cap (128 instructions,
 * two call sites) never admits it. The constant-trip tier admits it at call
 * sites that pass a constant 2..8 for the bounding parameter.
 *
 * This file pins the RESULT of every path the tier can take or must refuse:
 *   - constant trips inside and outside loops (inlined),
 *   - constants outside 2..8 (must stay calls: 0, 1, 9),
 *   - a non-constant trip count (stays a call),
 *   - an inclusive bound (`<=`, an off-by-one hazard),
 *   - a bound that the body reassigns before the loop,
 *   - an address-taken helper (its out-of-line body must survive).
 *
 * The expected values were computed with GCC 14 at -O0 (no inlining, no
 * optimization), so the oracle shares no code with the compiler under test.
 * The assembly-shape half of the contract is in
 * check_const_trip_loop_inline.sh.
 */
#include <stdint.h>
#include <stdio.h>

struct st {
    uint64_t a, b, c, d;
};

#define ROTL(x, r) (((x) << (r)) | ((x) >> (64 - (r))))

static void mix_rounds(struct st *s, const int rounds)
{
    for (int i = 0; i < rounds; i++) {
        s->a += s->b;
        s->b = ROTL(s->b, 13);
        s->b ^= s->a;
        s->a = ROTL(s->a, 32);
        s->c += s->d;
        s->d = ROTL(s->d, 16);
        s->d ^= s->c;
        s->a += s->d;
        s->d = ROTL(s->d, 21);
        s->d ^= s->a;
        s->c += s->b;
        s->b = ROTL(s->b, 17);
        s->b ^= s->c;
        s->c = ROTL(s->c, 32);
    }
}

/* Same body with an inclusive bound: runs rounds + 1 times. */
static void mix_rounds_le(struct st *s, const int rounds)
{
    for (int i = 0; i <= rounds; i++) {
        s->a += s->b;
        s->b = ROTL(s->b, 13);
        s->b ^= s->a;
        s->a = ROTL(s->a, 32);
        s->c += s->d;
        s->d = ROTL(s->d, 16);
        s->d ^= s->c;
        s->a += s->d;
        s->d = ROTL(s->d, 21);
        s->d ^= s->a;
        s->c += s->b;
        s->b = ROTL(s->b, 17);
        s->b ^= s->c;
        s->c = ROTL(s->c, 32);
    }
}

/* The bound is reassigned before the loop, so the exit compares a derived
 * value, not the incoming parameter. */
static void scale_rounds(struct st *s, int rounds)
{
    rounds = rounds * 2;
    for (int i = 0; i < rounds; i++) {
        s->a += s->b;
        s->b = ROTL(s->b, 13);
        s->b ^= s->a;
        s->a = ROTL(s->a, 32);
        s->c += s->d;
        s->d = ROTL(s->d, 16);
        s->d ^= s->c;
        s->a += s->d;
        s->d = ROTL(s->d, 21);
        s->d ^= s->a;
        s->c += s->b;
        s->b = ROTL(s->b, 17);
        s->b ^= s->c;
        s->c = ROTL(s->c, 32);
    }
}

/* The callers are noinline so each stays a small function of its own. If
 * they were inlined into main, main would exceed the 500-instruction caller
 * hard cap and the constant-trip clones would never be exercised here. */

/* Address-taken: the out-of-line body must still exist and be correct. */
static void (*volatile mix_rounds_ptr)(struct st *, const int) = mix_rounds;

__attribute__((noinline)) static uint64_t const_sites(struct st *s)
{
    mix_rounds(s, 2);
    for (int k = 0; k < 3; k++)
        mix_rounds(s, 4); /* constant site inside a caller loop */
    mix_rounds(s, 8);
    return s->a ^ s->d;
}

__attribute__((noinline)) static uint64_t out_of_range_sites(struct st *s)
{
    mix_rounds(s, 0);
    mix_rounds(s, 1);
    mix_rounds(s, 9);
    return s->a ^ s->d;
}

/* Alone in its caller, so the caller budget (350 instructions) admits the
 * clone and the largest admitted constant, 8, is actually exercised. */
__attribute__((noinline)) static uint64_t eight_site(struct st *s)
{
    mix_rounds(s, 8);
    return s->a ^ s->d;
}

__attribute__((noinline)) static uint64_t variable_site(struct st *s, int n)
{
    mix_rounds(s, n);
    return s->a ^ s->d;
}

__attribute__((noinline)) static uint64_t le_site(struct st *s)
{
    mix_rounds_le(s, 2);
    return s->a ^ s->d;
}

__attribute__((noinline)) static uint64_t scaled_site(struct st *s)
{
    scale_rounds(s, 2);
    return s->a ^ s->d;
}

__attribute__((noinline)) static uint64_t pointer_site(struct st *s)
{
    mix_rounds_ptr(s, 3);
    return s->a ^ s->d;
}

static void seed(struct st *s, uint64_t k)
{
    s->a = 0x736f6d6570736575ULL ^ k;
    s->b = 0x646f72616e646f6dULL ^ (k * 3);
    s->c = 0x6c7967656e657261ULL ^ (k * 5);
    s->d = 0x7465646279746573ULL ^ (k * 7);
}

static int failures;

static void check(const char *name, uint64_t got, uint64_t want)
{
    if (got != want) {
        printf("FAIL %s: got %016llx want %016llx\n", name,
               (unsigned long long)got, (unsigned long long)want);
        failures++;
    }
}

/* always_inline is honoured at a NON-constant site. The constant-trip tier
 * never admits always_inline bodies, so the clone must come from the normal
 * always_inline path, not from a constant argument. */
__attribute__((always_inline)) static void ai_mix(struct st *s, const int rounds)
{
    for (int i = 0; i < rounds; i++) {
        s->a += s->b;
        s->b = ROTL(s->b, 13);
        s->b ^= s->a;
        s->a = ROTL(s->a, 32);
        s->c += s->d;
        s->d = ROTL(s->d, 16);
        s->d ^= s->c;
        s->a += s->d;
        s->d = ROTL(s->d, 21);
        s->d ^= s->a;
        s->c += s->b;
        s->b = ROTL(s->b, 17);
        s->b ^= s->c;
        s->c = ROTL(s->c, 32);
    }
}

__attribute__((noinline)) static uint64_t always_variable_site(struct st *s, int n)
{
    ai_mix(s, n);
    return s->a ^ s->d;
}

int main(void)
{
    struct st s;

    seed(&s, 1);
    check("const_sites", const_sites(&s), UINT64_C(0xbc9d2f5364edb528));
    seed(&s, 2);
    check("out_of_range_sites", out_of_range_sites(&s), UINT64_C(0x49503734e6b59436));
    seed(&s, 3);
    check("variable_site(0)", variable_site(&s, 0), UINT64_C(0x070a090709070010));
    seed(&s, 4);
    check("variable_site(3)", variable_site(&s, 3), UINT64_C(0xc2063bfb56760121));
    seed(&s, 5);
    check("variable_site(-1)", variable_site(&s, -1), UINT64_C(0x070a090709070020));
    seed(&s, 6);
    check("le_site", le_site(&s), UINT64_C(0x7383ce76694830d2));
    seed(&s, 7);
    check("scaled_site", scaled_site(&s), UINT64_C(0x202f1cdb2b90c7c4));
    seed(&s, 8);
    check("pointer_site", pointer_site(&s), UINT64_C(0x030cfc30b617f289));
    seed(&s, 9);
    check("eight_site", eight_site(&s), UINT64_C(0xbf0888c527e9dd23));
    seed(&s, 10);
    check("always_variable_site", always_variable_site(&s, 5), UINT64_C(0xea63d0e16a578eee));

    if (failures) {
        printf("%d failure(s)\n", failures);
        return 1;
    }
    puts("ok");
    return 0;
}
