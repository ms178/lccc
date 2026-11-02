/* IPA-RA interposition regression (driver: check_ipa_ra_interposition.sh).
 * Each weak leaf below has a strong replacement in override.c with the same
 * value but a different register footprint (it clobbers %r8-%r11, which the
 * ABI allows).  A caller that kept a live value in one of those registers on
 * the strength of the weak body's IPA-RA summary computes a wrong result. */
__attribute__((noinline)) long leaf_pragma(long x) { return x * 3 + 1; }
#pragma weak leaf_pragma

__attribute__((noinline)) long leaf_late(long x) { return x * 3 + 1; }
long leaf_late(long) __attribute__((weak));

__attribute__((noinline)) long leaf_asm(long x) { return x * 3 + 1; }
__asm__(".weak leaf_asm");

/* Positive control: a static leaf binds locally, so its caller must use
 * the summary (the driver checks for an LCCC_IPA_KEEP marker). */
static __attribute__((noinline)) long leaf_static(long x) { return x * 3 + 1; }

#define USE(name, leaf)                                                    \
    long name(long a, long b, long c, long d) {                            \
        long s = a * b;                                                    \
        long t = c ^ d;                                                    \
        long r = leaf(a);                                                  \
        long q = leaf(b);                                                  \
        return s + t + r + q + a + b + c + d;                              \
    }
USE(use_pragma, leaf_pragma)
USE(use_late, leaf_late)
USE(use_asm, leaf_asm)
USE(use_static, leaf_static)
