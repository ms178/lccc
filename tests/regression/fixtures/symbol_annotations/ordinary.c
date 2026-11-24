/* One of everything an ordinary translation unit puts into its symbol table:
 * functions (exported, static, weak, variadic, a constructor), objects
 * (initialised, zeroed, read-only, thread-local, an array of pointers into
 * string literals), a switch that becomes a jump table, a computed goto,
 * floating constants, and 64-bit division, which is a runtime helper on
 * 32-bit targets. Compiled, never run: check_symbol_annotations.py reads
 * the symbol table. */
#include <stdarg.h>
/* No libc header: the cross targets are compiled without a sysroot. */
char *strncpy(char *, const char *, __SIZE_TYPE__);
struct pair { long a; double b; };
static int counter;
int exported = 7;
static const char *const names[] = {"zero", "one", "two"};
static __thread int tls_counter;
__thread long tls_exported = 3;
static const double weights[4] = {0.5, 1.5, 2.5, 3.5};
const int table[8] = {1, 2, 3, 4, 5, 6, 7, 8};
char buffer[64];
static struct pair zero_pair;
extern int external(int);
static int helper(int x) { return x * 3 + counter; }
int sum(int n, ...) {
    va_list ap; va_start(ap, n); int s = 0;
    for (int i = 0; i < n; i++) s += va_arg(ap, int);
    va_end(ap); return s;
}
double scale(double x) { return x * 1.25 + weights[(int)x & 3]; }
float fscale(float x) { return x * 2.5f; }
int dispatch(int k, int v) {
    switch (k) {
    case 0: return v + 1; case 1: return v * 2; case 2: return v - 3; case 3: return v ^ 5;
    case 4: return v << 1; case 5: return v >> 1; case 6: return -v; case 7: return ~v;
    default: return external(v);
    }
}
const char *name_of(unsigned i) { return i < 3 ? names[i] : "many"; }
int touch(void) { tls_counter++; tls_exported += tls_counter; counter += helper(exported); return counter; }
void fill(const char *s) { strncpy(buffer, s, sizeof buffer - 1); zero_pair.a++; }
void *label_addr(int i) { static void *const l[] = {&&a, &&b}; goto *l[i & 1]; a: return (void *)0; b: return buffer; }
long long wide(long long a, long long b) { return a / b + a % b; }
__attribute__((weak)) int weak_fn(void) { return 1; }
static void __attribute__((constructor)) init(void) { counter = 1; }
