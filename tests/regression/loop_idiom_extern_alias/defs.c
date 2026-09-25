/* Link X and Y as the same global, without exposing that fact to use.c.
 * The forward loop must propagate the first byte all the way to X[32]. */
unsigned char X[64];
extern unsigned char Y[64] __attribute__((alias("X")));
void copy_extern_alias(unsigned n);
int main(void) {
    for (unsigned i = 0; i < 64; ++i) X[i] = (unsigned char)(i + 1);
    copy_extern_alias(32);
    for (unsigned i = 0; i <= 32; ++i)
        if (X[i] != 1) return 1;
    return X[33] != 34;
}
