/* Separate TU: preserve the call/load effects of the OpenSSL-shaped join. */
unsigned *gf_names;
unsigned *gf_lock;
volatile unsigned gf_calls;
__attribute__((noinline)) void gf_cleanup(unsigned *p) {
    (void)p;
    gf_calls++;
}
