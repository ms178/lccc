/* -O1 companion of xmm_slot_gp_store_liveness.c: same peephole model bug,
 * exercised through the -O1 lowering shapes of gcc.c-torture 920428-2.c and
 * pr37573.c (unsigned<->float conversions and bit-level slot transfers). */
void abort(void);

__attribute__((noinline)) static unsigned f2u(float f) { return (unsigned) f; }
__attribute__((noinline)) static float u2f(unsigned u) { return (float) u; }

__attribute__((noinline)) static unsigned mix(unsigned a, float b, unsigned c)
{
    unsigned t = a * 2654435761u;      /* lives in a GP slot at -O1 */
    float g = u2f(t) + b;              /* transfer through the slot */
    unsigned r = f2u(g) ^ c;
    return r + t;
}

int main(void)
{
    if (f2u(4294967040.0f) != 4294967040u)
        abort();
    if (u2f(4294967295u) != 4294967296.0f)
        abort();
    if (mix(1u, 0.0f, 0u) != (unsigned)(2654435761u) + 2654435840u)
        abort();
    return 0;
}
