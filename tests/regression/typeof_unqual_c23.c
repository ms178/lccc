/* C23 typeof_unqual / __typeof_unqual__: yields the unqualified type.
 *
 * Kernel 6.18's per-CPU accessors (arch/x86/include/asm/percpu.h) wrap every
 * access in TYPEOF_UNQUAL() to strip the __seg_gs address-space qualifier;
 * without the keyword nothing including <asm/current.h> parses.
 *
 * The test checks the semantic contract, not just parsing: a variable
 * declared with __typeof_unqual__(const-qualified object) must be
 * assignable (the const must NOT propagate), and the type must otherwise
 * match exactly (size, signedness, pointer-ness).
 */
#include <stdio.h>

static const volatile int cv_src = 41;
static const char *const cp_src = "x";

int main(void)
{
       /* Qualifiers stripped: v is plain int, so writing it is legal. */
       __typeof_unqual__(cv_src) v;
       v = cv_src + 1;
       if (v != 42) {
               printf("FAIL qual strip v=%d\n", v);
               return 1;
       }

       /* typeof_unqual spelling (C23, gnu mode). */
       typeof_unqual(cv_src) w = 5;
       w += 1;
       if (w != 6) {
               printf("FAIL typeof_unqual spelling\n");
               return 2;
       }

       /* Top-level const on a pointer stripped: p reassignable; pointee
        * qualification is part of the pointed-to type and must survive. */
       __typeof_unqual__(cp_src) p = cp_src;
       p = "y";
       if (p[0] != 'y') {
               printf("FAIL pointer strip\n");
               return 3;
       }

       /* Size/signedness identity with the unqualified type. */
       _Static_assert(sizeof(__typeof_unqual__(cv_src)) == sizeof(int),
                      "size mismatch");
       __typeof_unqual__(cv_src) neg = -1;
       if (!(neg < 0)) {
               printf("FAIL signedness\n");
               return 4;
       }

       printf("PASS typeof_unqual_c23\n");
       return 0;
}
