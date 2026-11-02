/* under the PIC default, LOCAL TLS symbols (static
 * __thread) must use the Local-Exec model (direct %fs:TPOFF) exactly like
 * GCC; forcing the GOT-based GOTTPOFF sequence for a local symbol reads the
 * wrong slot (globals_tls returned 7: g_tls != 100).
 * This test needs default PIC (no -fno-pic): a static __thread read must
 * return its initialized value. */
#include <stdio.h>

static __thread int t_a = 100;
static __thread long t_b = 0x123456789;

int main(void) {
    if (t_a != 100) { printf("FAIL t_a=%d\n", t_a); return 1; }
    if (t_b != 0x123456789L) { printf("FAIL t_b=%ld\n", t_b); return 2; }
    t_a = 42;
    if (t_a != 42) { printf("FAIL t_a write\n"); return 3; }
    printf("PASS tls_local_pic\n");
    return 0;
}
