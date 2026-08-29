/* rv64 canonical-form discipline for neg/not across integer widths.
 *
 * Invariant (rv64 codegen): every <=32-bit integer in a 64-bit register
 * equals signext(low32); signed narrow values are additionally sign-filled
 * to their own width, unsigned narrow/U32 keep an explicit zero-extension.
 * I32 neg uses the *w form (negw) and I32 not is the plain 64-bit `not`
 * (xori -1) — both GCC-identical; the old unconditional zext pairs
 * violated the invariant and the sub-width refills are load-bearing for
 * later peepholes (sext-wide producers no longer include slli/srli).
 *
 * Differential: compiled by lccc-riscv AND riscv64 gcc; the runner
 * requires identical stdout and exit code.
 */

volatile int vi = -3;
volatile long long vll = -5;
volatile unsigned vu = 7;

static long long n64(long long x) { return -x; }
static long long c64(long long x) { return ~x; }
static int n32(int x) { return -x; }
static int c32(int x) { return ~x; }
static short n16(short x) { return -x; }
static short c16(short x) { return ~x; }
static char n8(char x) { return -x; }
static char c8(char x) { return ~x; }
static unsigned int n32u(unsigned int x) { return -x; }
static unsigned int c32u(unsigned int x) { return ~x; }
static unsigned short n16u(unsigned short x) { return -x; }
static unsigned short c16u(unsigned short x) { return ~x; }

int main(void) {
    /* loop/iterative caller shape: hides nothing from the canonical-form
     * check (a caller-side (long long)(-i32) bug only shows here) */
    long long acc = 0;
    for (int i = 0; i < 8; i++) {
        acc = acc * 3 + n64(-(long long)i);
        acc = acc ^ c64((long long)i * -2);
        acc += n64(vll);
    }
    __builtin_printf("acc=%lld\n", acc);

    int a = 0;
    for (int i = 0; i < 5; i++) {
        a = a * 7 + n32(i * 1000) + c32(i);
        a += (int)(short) n16((short)(i * 3000));
        a += (int)(char) n8((char)(i * 70));
        a += (int) n32u(vu + (unsigned)i);
        a += (int) c32u(vu + (unsigned)i * 100000u);
        a += (int) n16u((unsigned short)(i * 20000));
        a += (int) c16u((unsigned short)(i * 9000));
    }
    __builtin_printf("a=%d\n", a);

    /* direct unary shapes at all widths */
    __builtin_printf("%d %d %d %d\n", n32(123456), c32(123456), n32(-1), c32(-1));
    __builtin_printf("%d %d %d %d\n", n16(-3), c16(-3), n8(-3), c8(-3));
    __builtin_printf("%u %u\n", n32u(7), c32u(7));
    __builtin_printf("%lld %lld\n", n64(-1LL), c64(-1LL));
    return 0;
}
