/* Freestanding mix for check_riscv_relax_bfd.py: the code shapes GCC emits
 * relaxable relocations for, linked by lccc-ld and GNU ld 2.47 and compared
 * byte for byte.  Small and large data on both sides of gp, small FP
 * constants (.srodata.cst8), local-exec TLS, near calls and tail calls, a
 * jump table and aligned loops (R_RISCV_ALIGN at -O2).  Never executed:
 * only the link is under test. */

int big_data[600] = {1, 2, 3};
int small_init = 5;
static int small_zero;
static long counters[4];
int big_bss[2048];
const char *greeting = "relax";
__thread int tls_small;
__thread int tls_array[1024];
__thread int tls_far;
double scale = 1.5;

__attribute__((noinline)) static int leaf(int x) { return x + small_init + small_zero; }

__attribute__((noinline)) int touch(int i) {
    counters[i & 3]++;
    big_bss[i & 2047] = i;
    return big_data[(i * 7) % 600] + leaf(i);
}

__attribute__((noinline)) double fp(double x) { return x * scale + 0.25; }

__attribute__((noinline)) int dispatch(int op, int v) {
    switch (op) {
    case 0: return v + 1;
    case 1: return v * 3;
    case 2: return v - small_init;
    case 3: return touch(v);
    case 4: return v ^ 0x55;
    case 5: return (int)fp(v);
    case 6: return greeting[v & 3];
    default: return -v;
    }
}

__attribute__((noinline)) int tls_work(int i) {
    tls_small += i;
    tls_array[i & 1023] = tls_small;
    tls_far = tls_array[(i + 1) & 1023];
    return tls_far;
}

__attribute__((noinline)) int loop(int n) {
    int acc = 0;
    for (int i = 0; i < n; i++)
        acc += dispatch(i & 7, acc) + tls_work(i);
    return acc;
}

__attribute__((noinline)) int tail(int n) { return loop(n + 1); }

void _start(void) {
    small_zero = tail(3);
    for (;;)
        ;
}
