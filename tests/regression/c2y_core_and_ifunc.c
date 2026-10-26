/* C23/C2y core language + headers + cleanup LIFO + VLA={} + local IFUNC. */
#if __has_include(<stdckdint.h>)
#include <stdckdint.h>
#else
#include "../../include/stdckdint.h"
#endif
#if __has_include(<stdcountof.h>)
#include <stdcountof.h>
#else
#include "../../include/stdcountof.h"
#endif
#include <stdint.h>
#include <stdio.h>

#if 0o77 != 63
#error "C2y 0o octal failed in #if"
#endif

#if '\o{101}' != 'A' || '\x{42}' != 'B'
#error "C2y delimited escapes failed in #if"
#endif

static int g_cleanup_log[16];
static int g_cleanup_n;

static void record_cleanup(int *p) {
    if (g_cleanup_n < 16) {
        g_cleanup_log[g_cleanup_n++] = *p;
    }
}

[[maybe_unused]] static int pure_add_u32(int a, int b) [[unsequenced]] {
    return a + b;
}

static int read_slot(const int *p) [[reproducible]] {
    return *p;
}

[[gnu::noinline]] static int run_cleanup_lifo(int mode) {
    int a __attribute__((cleanup(record_cleanup))) = 10;
    {
        int b __attribute__((cleanup(record_cleanup))) = 20;
        if (mode == 1) {
            return a + b;
        }
        int c __attribute__((cleanup(record_cleanup))) = 30;
        (void)c;
    }
    for (int i = 0; i < 3; ++i) {
        int d __attribute__((cleanup(record_cleanup))) = 100 + i;
        if (i == 1) break;
        (void)d;
    }
    return a;
}

static int impl_fast(int x) { return x * 3 + 7; }
static int (*resolve_compute(void))(int) { return impl_fast; }
static int local_ifunc_compute(int x) __attribute__((ifunc("resolve_compute")));

__attribute__((noinline)) static int check_vla_zero(int n) {
    int vla[n] = {};
    int sum = 0;
    for (int i = 0; i < n; ++i) sum += vla[i];
    return sum;
}

int main(void) {
    /* 0o/0O, wb/uwb, delimited escapes, _Countof / countof */
    int arr[0o11];
    _Static_assert(_Countof(arr) == 9, "_Countof");
    _Static_assert(countof(arr) == 9, "countof");
    auto bi = 0o12345uwb;
    const char *s = "\o{110}\x{65}llo-\u{0041}";
    if (s[0] != 'H' || s[1] != 'e' || s[6] != 'A') return 1;

    /* <stdckdint.h> */
    uint32_t out32 = 0;
    if (ckd_add(&out32, 0xfffffff0u, 0x20u) != 1) return 2;
    if (out32 != 0x10u) return 3;
    if (ckd_mul(&out32, 7u, 9u) != 0 || out32 != 63u) return 4;

    /* C2y if / switch init-statements + labeled break / continue */
    int acc = 0;
    if (auto k = 0o20; k == 16) {
        acc += k;
    }
    switch (int sw = 4; sw + 1) {
        case 5: acc += 100; break;
        default: return 5;
    }

outer_loop:
    for (int i = 0; i < 5; ++i) {
        for (int j = 0; j < 5; ++j) {
            if (j == 2 && i < 3) continue outer_loop;
            if (i == 3 && j == 1) break outer_loop;
            acc += 1;
        }
    }

    /* cleanup LIFO order + VLA = {} + attributes + local IFUNC */
    g_cleanup_n = 0;
    int r1 = run_cleanup_lifo(1);
    int r2 = run_cleanup_lifo(0);
    if (r1 != 30 || r2 != 10) return 6;
    if (check_vla_zero(23) != 0) return 7;

    int slot = local_ifunc_compute(11);
    int total = pure_add_u32(read_slot(&slot), acc) + (int)bi;

    printf("core_ifunc=%d cleanups=%d:", total, g_cleanup_n);
    for (int i = 0; i < g_cleanup_n; ++i) {
        printf(" %d", g_cleanup_log[i]);
    }
    printf("\n");
    return 0;
}
