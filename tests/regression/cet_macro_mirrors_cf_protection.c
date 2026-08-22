/* __CET__ must mirror -fcf-protection exactly (GCC parity).
 *
 * GCC defines __CET__ ONLY under an active -fcf-protection: =branch -> 1,
 * =return -> 2, =full/bare -> 3; =none or absence leaves it undefined.
 * lccc used to predefine __CET__=3 unconditionally, which dragged
 * CET-only code into builds that disabled it: glibc --disable-cet compiles
 * rtld WITHOUT dl-cet.c, but sysdeps/x86_64/sysdep.h saw __CET__ and made
 * dl_main call _dl_cet_check/_dl_cet_setup_features — undefined symbols at
 * the ld.so link.
 *
 * This test's .flags carry no -fcf-protection, so __CET__ must be absent.
 */
#include <stdio.h>

#ifdef __CET__
#error "__CET__ must not be predefined without -fcf-protection"
#endif

int main(void)
{
    printf("cet:absent\n");
    return 0;
}
