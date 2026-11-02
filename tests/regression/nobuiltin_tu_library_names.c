/* -fno-builtin: every library name means what THIS program defines, so no
 * call may be folded, mapped to an instruction or expanded inline from the
 * library contract.  Each function below deliberately breaks that contract
 * (legal: -fno-builtin is the documented way to own these names), and each
 * line of output is what a library-contract rewrite would get wrong:
 *   strlen("abc") folded to 3     sqrt -> sqrtsd      pow(x, 2) -> x*x
 *   fixed-size memcpy expanded    fabs -> andpd
 * Compared against GCC 16.2 (same flags).
 */
typedef __SIZE_TYPE__ size_t;
int printf(const char *, ...);

static int copies;

size_t strlen(const char *s) {
    size_t n = 100;
    while (*s++) n++;
    return n;
}
double sqrt(double x) { return x + 1.0; }
double pow(double x, double y) { return x + y; }
double fabs(double x) { return x - 1.0; }
void *memcpy(void *d, const void *s, size_t n) {
    copies++;
    unsigned char *p = d;
    const unsigned char *q = s;
    for (size_t i = 0; i < n; i++) p[i] = q[i];
    return d;
}

int main(void) {
    volatile double x = 4.0;
    long long a = 0x1122334455667788LL, b = 0;
    memcpy(&b, &a, sizeof b);
    printf("%zu %.1f %.1f %.1f %d %llx\n", strlen("abc"), sqrt(x), pow(x, 2.0),
           fabs(x), copies, (unsigned long long)b);
    return 0;
}
