/* gcc.c-torture pr126405-3, reduced to the aggregate-copy snapshot contract.
 * old must retain the incoming value after p is overwritten from g0.  A
 * later copy with an untracked global source is still a definition of p;
 * aggregate forwarding must not redirect old's reads to the original input.
 * long long keeps the oversized vector 128 bytes on both x86 ABIs. */
typedef long long v16di __attribute__((vector_size(16 * sizeof(long long))));

v16di g0, g1;

__attribute__((noinline)) static void dirty_stack(void) {
    volatile char buf[1024];
    for (unsigned i = 0; i < sizeof(buf); i++)
        buf[i] = 0xa5;
}

/* Deliberately inlinable: the original failure appeared after inlining. */
static v16di snapshot_sum(v16di p) {
    v16di old = p;
    p = g0;
    g1 = p;
    return old + p;
}

int main(void) {
    v16di a, r;
    for (int i = 0; i < 16; i++) {
        a[i] = i + 1;
        g0[i] = 100;
    }
    dirty_stack();
    r = snapshot_sum(a);
    for (int i = 0; i < 16; i++)
        if (r[i] != i + 101 || g1[i] != 100)
            __builtin_abort();
    return 0;
}
